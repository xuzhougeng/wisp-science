//! Native ZINC domain against CartBlanche22 (ZINC22) and the ZINC-22 file
//! repository. Independently implemented from:
//!
//! - [SmallWorld public API](https://wiki.docking.org/index.php?title=How_to_use_SmallWorld_API)
//! - [ZINC22 searching](https://wiki.docking.org/index.php?title=Zinc22:Searching)
//! - [ZINC22 numbering](https://wiki.docking.org/index.php?title=ZINC22:Numbering)
//! - [ZINC22 directory structure](https://wiki.docking.org/index.php/ZINC22:Directory_structure)
//! - [CartBlanche22](https://cartblanche22.docking.org/)
//! - [files.docking.org/zinc22](https://files.docking.org/zinc22/)
//!
//! References reviewed 2026-09-06. CartBlanche22 search endpoints accept a
//! form-encoded POST and return a Celery task receipt `{"task": "<uuid>"}`;
//! results are read from `GET /search/result/<uuid>`. GET query-string search
//! is not the API (it serves the HTML app). No API key is published.
//! Tests use invented records.

#[cfg(test)]
mod tests;

use crate::http::Source;
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};
use wisp_llm::ToolSchema;

const CARTBLANCHE: &str = "https://cartblanche22.docking.org";
const FILES: &str = "https://files.docking.org/zinc22";
const SEARCH_FIELDS: &str = "zinc_id,smiles,tranche_name,catalogs";
const SUPPLIER_FIELDS: &str = "zinc_id,smiles,supplier_code,catalogs,tranche_name";
const ZINC: Source = Source("ZINC", Duration::from_millis(300));
const DEFAULT_MAX: u32 = 50;
const MAX_RESULTS: u32 = 500;
const MAX_IDS: usize = 100;
const MAX_IDS_3D: usize = 50;
const DEFAULT_TIMEOUT: f64 = 25.0;
const MIN_TIMEOUT: f64 = 1.0;
const MAX_TIMEOUT: f64 = 45.0;
const MAX_SMILES: usize = 4096;

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        (
            "zinc",
            ToolSchema::new(
                "zinc_search_by_id",
                "Look up purchasable ZINC22/ZINC20 compounds by ZINC identifier through CartBlanche22. Submits a form POST to /substances.txt and waits for GET /search/result/{task}. Returns a bounded page of SMILES, vendor catalogs, tranche properties and source URLs. Identifiers with no match are listed in missing_ids; a capped page is not the complete hit list.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["zinc_ids"],
                    "properties": {
                        "zinc_ids": {
                            "type": "array", "minItems": 1, "maxItems": 100,
                            "items": {"type": "string", "minLength": 5, "maxLength": 24,
                                "pattern": "^ZINC[0-9A-Za-z]+$"}
                        },
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 500, "default": 50},
                        "timeout_s": {"type": "number", "minimum": 1, "maximum": 45, "default": 25}
                    }
                }),
            ),
        ),
        (
            "zinc",
            ToolSchema::new(
                "zinc_search_by_smiles",
                "Search the active public SmallWorld ZINC20 for-sale index by SMILES. dist limits scored graph-edit distance and adist limits anonymous-graph distance (both default to 0 for exact matching). Returns a bounded hit page, distances, ZINC identifiers and the actual index name; use zinc_search_by_id for vendor details. Coverage is the named ZINC20 index, not the entire ZINC22 space.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["smiles"],
                    "properties": {
                        "smiles": {"type": "string", "minLength": 1, "maxLength": 4096},
                        "dist": {"type": "integer", "minimum": 0, "maximum": 10, "default": 0},
                        "adist": {"type": "integer", "minimum": 0, "maximum": 10, "default": 0},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 500, "default": 50},
                        "timeout_s": {"type": "number", "minimum": 1, "maximum": 45, "default": 25}
                    }
                }),
            ),
        ),
        (
            "zinc",
            ToolSchema::new(
                "zinc_search_by_supplier",
                "Resolve vendor catalog numbers to ZINC substances through CartBlanche22 /catitems.txt. Returns matching ZINC identifiers, SMILES, catalogs and the supplier_code that matched. Supplier codes are case-sensitive; use their exact spelling from zinc_search_by_id catalogs. Codes with no match are listed in missing_ids.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["supplier_codes"],
                    "properties": {
                        "supplier_codes": {
                            "type": "array", "minItems": 1, "maxItems": 100,
                            "items": {"type": "string", "minLength": 1, "maxLength": 128}
                        },
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 500, "default": 50},
                        "timeout_s": {"type": "number", "minimum": 1, "maximum": 45, "default": 25}
                    }
                }),
            ),
        ),
        (
            "zinc",
            ToolSchema::new(
                "zinc_get_3d",
                "Locate docking-ready 3D conformers for ZINC compounds. Looks up each identifier and maps its HAC/logP tranche onto the files.docking.org/zinc22 repository (generations zinc-22a…zinc-22z, formats db2/mol2/sdf/pdbqt). Does not download archives. Missing identifiers are reported individually. At most 50 identifiers per call.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["zinc_ids"],
                    "properties": {
                        "zinc_ids": {
                            "type": "array", "minItems": 1, "maxItems": 50,
                            "items": {"type": "string", "minLength": 5, "maxLength": 24,
                                "pattern": "^ZINC[0-9A-Za-z]+$"}
                        },
                        "timeout_s": {"type": "number", "minimum": 1, "maximum": 45, "default": 25}
                    }
                }),
            ),
        ),
        (
            "zinc",
            ToolSchema::new(
                "zinc_random_sample",
                "Draw a random sample of purchasable ZINC22 compounds through CartBlanche22 /substance/random.json and its dedicated /substance/random/{task}.json polling route. count is the sample size and the response bound (1–500). subset, when set, is forwarded; CartBlanche22 documents lead-like as a predefined property filter. Each call draws a fresh sample.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "count": {"type": "integer", "minimum": 1, "maximum": 500, "default": 50},
                        "subset": {"type": "string", "minLength": 1, "maxLength": 64},
                        "timeout_s": {"type": "number", "minimum": 1, "maximum": 45, "default": 25}
                    }
                }),
            ),
        ),
    ]
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "zinc_search_by_id" => search_by_id(bio, args).await,
        "zinc_search_by_smiles" => search_by_smiles(bio, args).await,
        "zinc_search_by_supplier" => search_by_supplier(bio, args).await,
        "zinc_get_3d" => get_3d(bio, args).await,
        "zinc_random_sample" => random_sample(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchById {
    zinc_ids: Vec<String>,
    #[serde(default = "default_max")]
    max_results: u32,
    #[serde(default = "default_timeout")]
    timeout_s: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchBySmiles {
    smiles: String,
    #[serde(default)]
    dist: i64,
    adist: Option<i64>,
    #[serde(default = "default_max")]
    max_results: u32,
    #[serde(default = "default_timeout")]
    timeout_s: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchBySupplier {
    supplier_codes: Vec<String>,
    #[serde(default = "default_max")]
    max_results: u32,
    #[serde(default = "default_timeout")]
    timeout_s: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Get3d {
    zinc_ids: Vec<String>,
    #[serde(default = "default_timeout")]
    timeout_s: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RandomSample {
    #[serde(default = "default_max")]
    count: u32,
    subset: Option<String>,
    #[serde(default = "default_timeout")]
    timeout_s: f64,
}

fn default_max() -> u32 {
    DEFAULT_MAX
}

fn default_timeout() -> f64 {
    DEFAULT_TIMEOUT
}

async fn search_by_id(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchById =
        serde_json::from_value(args.clone()).context("invalid ZINC identifier search arguments")?;
    let ids = require_ids(&args.zinc_ids, MAX_IDS, "ZINC id", true)?;
    let cap = bound_page(args.max_results)?;
    let timeout = clamp_timeout(args.timeout_s)?;
    let result = search(
        bio,
        "substances.txt",
        vec![
            ("zinc_ids".into(), ids.join(",")),
            ("output_fields".into(), SEARCH_FIELDS.into()),
        ],
        timeout,
    )
    .await?;
    let (records, counts) = flatten_result(&result)?;
    let missing = missing_zinc_ids(&ids, &records);
    Ok(page(
        records,
        counts,
        cap,
        json!({"zinc_ids": ids}),
        missing,
    ))
}

async fn search_by_smiles(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchBySmiles =
        serde_json::from_value(args.clone()).context("invalid ZINC SMILES search arguments")?;
    let smiles = args.smiles.trim();
    if smiles.is_empty() || smiles.len() > MAX_SMILES {
        bail!("smiles must contain 1 to {MAX_SMILES} characters");
    }
    if !(0..=10).contains(&args.dist) {
        bail!("dist must be 0–10 (0 is an exact structure match)");
    }
    let adist = args.adist.unwrap_or(0);
    if !(0..=10).contains(&adist) {
        bail!("adist must be 0–10 (anonymous-graph distance)");
    }
    let cap = bound_page(args.max_results)?;
    let timeout = clamp_timeout(args.timeout_s)?;
    smallworld_search(bio, smiles, args.dist, adist, cap, timeout).await
}

// The public SmallWorld service exposes a maintained ZINC20 for-sale index.
// CartBlanche's internal ZINC22 SMILES worker can complete with an empty input
// marker; querying the documented public service gives explicit search state,
// a bounded hit page and a named index in every result.
async fn smallworld_search(
    bio: &NativeBio,
    smiles: &str,
    dist: i64,
    adist: i64,
    cap: usize,
    timeout: f64,
) -> Result<Value> {
    let base = bio
        .credential("ZINC_SMALLWORLD_URL")
        .unwrap_or("https://sw.docking.org")
        .trim_end_matches('/');
    let maps = zinc_json(bio, Method::GET, &format!("{base}/search/maps"), &[]).await?;
    let (index, _) = maps
        .as_object()
        .context("SmallWorld omitted its index catalog")?
        .iter()
        .filter(|(id, value)| {
            id.to_ascii_lowercase().starts_with("zinc20-forsale-") && value["enabled"] == true
        })
        .max_by_key(|(id, _)| *id)
        .context("SmallWorld has no enabled ZINC20 for-sale index")?;
    let params = vec![
        ("smi".into(), smiles.into()),
        ("db".into(), index.clone()),
        ("fmt".into(), "json".into()),
        ("start".into(), "0".into()),
        ("length".into(), (cap + 1).to_string()),
        ("dist".into(), adist.to_string()),
        ("sdist".into(), dist.to_string()),
        ("scores".into(), "AtomAlignment".into()),
        ("async".into(), "true".into()),
    ];
    let deadline = Instant::now() + Duration::from_secs_f64(timeout);
    loop {
        if Instant::now() >= deadline {
            bail!("SmallWorld search did not complete within {timeout:.0}s");
        }
        let value = zinc_json(bio, Method::GET, &format!("{base}/search/view"), &params).await?;
        match value.pointer("/status/state").and_then(Value::as_str) {
            Some("RUNNING" | "QUEUED") => {
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }
            Some("DONE") => {}
            _ => bail!("SmallWorld returned an unsuccessful search state"),
        }
        let total = value["recordsFiltered"]
            .as_u64()
            .context("SmallWorld omitted its match count")?;
        let hits = value["data"]
            .as_array()
            .context("SmallWorld omitted its hit list")?;
        if total > 0 && hits.is_empty() {
            bail!("SmallWorld omitted matching records");
        }
        let mut records = Vec::new();
        for hit in hits.iter().take(cap) {
            let identity = hit.get(0).context("SmallWorld returned an invalid hit")?;
            let id = identity["id"]
                .as_str()
                .context("SmallWorld omitted a ZINC identifier")?;
            let id = if id.bytes().all(|b| b.is_ascii_digit()) && !id.is_empty() {
                format!("ZINC{id:0>12}")
            } else {
                id.to_string()
            };
            if !is_zinc_id(&id) {
                bail!("SmallWorld returned an invalid ZINC identifier");
            }
            let smiles = identity["hitSmiles"]
                .as_str()
                .and_then(|s| s.split_whitespace().next())
                .context("SmallWorld omitted the hit structure")?;
            records.push(json!({"zinc_id":id,"smiles":smiles,"source":"zinc20","url":compound_url(&id),"distance":hit.get(1),"alignment_distance":hit.get(2)}));
        }
        return Ok(json!({
            "source":"ZINC SmallWorld", "source_url":"https://sw.docking.org", "index":index,
            "query":{"smiles":smiles,"dist":dist,"adist":adist},
            "total_available":total, "returned":records.len(), "truncated":total > records.len() as u64,
            "source_counts":{"zinc20":total}, "missing_ids":[], "records":records,
        }));
    }
}

async fn search_by_supplier(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchBySupplier =
        serde_json::from_value(args.clone()).context("invalid ZINC supplier search arguments")?;
    let codes = require_ids(&args.supplier_codes, MAX_IDS, "supplier code", false)?;
    let cap = bound_page(args.max_results)?;
    let timeout = clamp_timeout(args.timeout_s)?;
    let result = search(
        bio,
        "catitems.txt",
        vec![
            ("supplier_codes".into(), codes.join(",")),
            ("output_fields".into(), SUPPLIER_FIELDS.into()),
        ],
        timeout,
    )
    .await?;
    let (records, counts) = flatten_result(&result)?;
    let missing = missing_supplier_codes(&codes, &records);
    Ok(page(
        records,
        counts,
        cap,
        json!({"supplier_codes": codes}),
        missing,
    ))
}

async fn random_sample(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: RandomSample =
        serde_json::from_value(args.clone()).context("invalid ZINC random sample arguments")?;
    let cap = bound_page(args.count)?;
    let timeout = clamp_timeout(args.timeout_s)?;
    let subset = args
        .subset
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(name) = subset {
        if name.len() > 64 || name.chars().any(|c| c == ',' || c.is_whitespace()) {
            bail!("subset must be a single CartBlanche22 subset name without whitespace");
        }
    }
    let mut form = vec![
        ("count".into(), cap.to_string()),
        ("output_fields".into(), SEARCH_FIELDS.into()),
    ];
    if let Some(name) = subset {
        form.push(("subset".into(), name.to_string()));
    }
    let result = search(bio, "substance/random.json", form, timeout).await?;
    let result = match result {
        Value::String(text) => {
            serde_json::from_str(&text).context("ZINC random sample returned invalid JSON")?
        }
        value => value,
    };
    let (records, counts) = flatten_result(&result)?;
    if records.is_empty() {
        bail!("ZINC did not return any compounds for the random sample");
    }
    Ok(page(
        records,
        counts,
        cap,
        json!({"count": cap, "subset": subset}),
        Vec::new(),
    ))
}

async fn get_3d(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Get3d =
        serde_json::from_value(args.clone()).context("invalid ZINC 3D lookup arguments")?;
    let ids = require_ids(&args.zinc_ids, MAX_IDS_3D, "ZINC id", true)?;
    let canonical: Vec<String> = ids.iter().map(|id| canonical_zinc_id(id)).collect();
    let timeout = clamp_timeout(args.timeout_s)?;
    let result = search(
        bio,
        "substances.txt",
        vec![
            ("zinc_ids".into(), canonical.join(",")),
            ("output_fields".into(), SEARCH_FIELDS.into()),
        ],
        timeout,
    )
    .await?;
    let (records, _counts) = flatten_result(&result)?;
    let mut by_id = BTreeMap::new();
    for record in &records {
        if let Some(zinc_id) = record.get("zinc_id").and_then(Value::as_str) {
            by_id
                .entry(canonical_zinc_id(zinc_id))
                .or_insert_with(|| record.clone());
        }
    }
    let mut structures = Vec::new();
    let mut missing = Vec::new();
    for (original, canon) in ids.iter().zip(canonical.iter()) {
        let Some(record) = by_id.get(canon) else {
            missing.push(original.clone());
            structures.push(json!({"zinc_id": original, "found": false}));
            continue;
        };
        let zinc_id = record
            .get("zinc_id")
            .and_then(Value::as_str)
            .unwrap_or(canon);
        let parsed = record
            .get("tranche_name")
            .and_then(Value::as_str)
            .and_then(parse_tranche);
        let mut entry = json!({
            "zinc_id": zinc_id,
            "found": true,
            "smiles": record.get("smiles"),
            "source": record.get("source"),
            "url": compound_url(zinc_id),
        });
        if let Some((heavy_atoms, logp, tranche)) = parsed {
            entry["tranche_name"] = json!(tranche);
            entry["tranche_properties"] = json!({"heavy_atoms": heavy_atoms, "logp": logp});
            entry["download"] = json!({
                "repository": format!("{FILES}/"),
                "tranche_path_pattern": format!(
                    "zinc-22*/H{heavy_atoms:02}/{tranche}/"
                ),
                "formats": {
                    "db2.tgz": "DOCK 3.x/6 multi-conformer database",
                    "mol2.tgz": "Tripos MOL2 with 3D coordinates",
                    "sdf.tgz": "SDF with 3D coordinates",
                    "pdbqt.tgz": "AutoDock PDBQT"
                }
            });
        }
        structures.push(entry);
    }
    Ok(json!({
        "source": "ZINC CartBlanche22",
        "source_url": CARTBLANCHE,
        "files_url": format!("{FILES}/"),
        "query": {"zinc_ids": ids},
        "returned": structures.len(),
        "missing_ids": missing,
        "structures": structures,
        "repository_note": "3D files are tranche archives under https://files.docking.org/zinc22/, grouped by generation (zinc-22a … zinc-22z), heavy-atom count and logP bin. Browse generation directories for exact file names; CartBlanche22 does not expose a per-compound fetch URL."
    }))
}

async fn search(
    bio: &NativeBio,
    endpoint: &str,
    form: Vec<(String, String)>,
    timeout_s: f64,
) -> Result<Value> {
    let base = api_base(bio);
    let deadline = Instant::now() + Duration::from_secs_f64(timeout_s);
    let submit = zinc_json(bio, Method::POST, &format!("{base}/{endpoint}"), &form).await?;
    if let Some(result) = immediate_result(&submit) {
        return Ok(result);
    }
    let task = task_id(&submit).with_context(|| {
        format!("ZINC {endpoint} response carried no task id (async submit contract)")
    })?;
    if task.len() > 80 || task.chars().any(|c| c.is_whitespace() || c == '/') {
        bail!("ZINC {endpoint} returned an invalid task id");
    }
    let poll_path = if endpoint == "substance/random.json" {
        format!("/substance/random/{}.json", path_segment(&task))
    } else {
        format!("/search/result/{}", path_segment(&task))
    };
    let poll_url = format!("{base}{poll_path}");
    loop {
        if Instant::now() >= deadline {
            bail!(
                "ZINC task {task} did not complete within {timeout_s:.0}s. \
                 Re-poll {CARTBLANCHE}{poll_path} later, or retry with a narrower query."
            );
        }
        let payload = zinc_json(bio, Method::GET, &poll_url, &[]).await?;
        match classify_poll(&payload, &task)? {
            Poll::Ready(result) => return Ok(result),
            Poll::Pending => {}
        }
    }
}

enum Poll {
    Ready(Value),
    Pending,
}

fn classify_poll(payload: &Value, task: &str) -> Result<Poll> {
    let status = payload.get("status").and_then(Value::as_str);
    match status {
        Some("FAILURE") | Some("ERROR") => {
            bail!("ZINC task {task} failed server-side (status FAILURE). Check SMILES syntax and identifier formats.")
        }
        Some("SUCCESS") => Ok(Poll::Ready(
            payload.get("result").cloned().unwrap_or(Value::Null),
        )),
        Some("PENDING" | "STARTED" | "PROGRESS" | "RETRY") => Ok(Poll::Pending),
        None if payload.get("result").is_some() => Ok(Poll::Ready(payload["result"].clone())),
        None => Ok(Poll::Pending),
        Some(other) => bail!("ZINC task {task} reported unexpected status {other}"),
    }
}

fn immediate_result(payload: &Value) -> Option<Value> {
    if task_id(payload).is_some() {
        return None;
    }
    if payload.get("status").and_then(Value::as_str) == Some("SUCCESS")
        || payload.get("result").is_some()
    {
        return Some(payload.get("result").cloned().unwrap_or(Value::Null));
    }
    None
}

fn task_id(payload: &Value) -> Option<String> {
    match payload.get("task") {
        Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
        Some(Value::Number(number)) => Some(number.to_string()),
        _ => None,
    }
}

async fn zinc_json(
    bio: &NativeBio,
    method: Method,
    url: &str,
    params: &[(String, String)],
) -> Result<Value> {
    let response = bio.http().send(ZINC, method, url, params).await?;
    response.check()?;
    if looks_like_html(&response.body) {
        bail!(
            "ZINC returned its HTML app shell instead of JSON — the request was not understood as an API call (use form POST, not a GET query string)"
        );
    }
    serde_json::from_slice(&response.body).context("ZINC returned invalid JSON")
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

fn flatten_result(result: &Value) -> Result<(Vec<Value>, BTreeMap<String, usize>)> {
    let buckets = match result {
        Value::Null => Vec::new(),
        Value::Array(rows) => vec![("zinc22".to_string(), rows.clone())],
        Value::Object(map) => {
            let mut buckets = Vec::new();
            for key in ["zinc22", "zinc20"] {
                if let Some(Value::Array(rows)) = map.get(key) {
                    buckets.push((key.to_string(), rows.clone()));
                }
            }
            let mut extra: Vec<_> = map
                .iter()
                .filter(|(key, value)| {
                    !matches!(
                        key.as_str(),
                        "zinc22" | "zinc20" | "missing" | "zinc22_missing" | "submission"
                    ) && value.is_array()
                })
                .collect();
            extra.sort_by(|a, b| a.0.cmp(b.0));
            for (key, value) in extra {
                if let Value::Array(rows) = value {
                    buckets.push((key.clone(), rows.clone()));
                }
            }
            buckets
        }
        _ => bail!("ZINC produced an unrecognized result shape (expected a dict keyed by source)"),
    };
    let mut records = Vec::new();
    let mut counts = BTreeMap::new();
    for (source, rows) in buckets {
        let mut n = 0usize;
        for row in rows {
            if let Value::Object(_) = &row {
                if let Some(projected) = project_record(&row, &source) {
                    records.push(projected);
                    n += 1;
                }
            }
        }
        counts.insert(source, n);
    }
    Ok((records, counts))
}

fn project_record(raw: &Value, bucket: &str) -> Option<Value> {
    let zinc_id = record_string(raw, &["zinc_id", "zincid", "ZINC_ID"])?;
    let source = record_string(raw, &["db", "source"]).unwrap_or_else(|| bucket.to_string());
    let smiles = record_string(raw, &["smiles", "SMILES"]);
    let tranche = tranche_code(raw);
    let props = raw
        .get("tranche_details")
        .and_then(tranche_details)
        .or_else(|| {
            tranche
                .as_deref()
                .and_then(parse_tranche)
                .map(|(heavy_atoms, logp, _)| json!({"heavy_atoms": heavy_atoms, "logp": logp}))
        });
    let mut record = json!({
        "zinc_id": zinc_id,
        "smiles": smiles,
        "source": source,
        "url": compound_url(&zinc_id),
        "catalogs": raw.get("catalogs").cloned().unwrap_or(Value::Null),
    });
    if let Some(code) = raw.get("supplier_code") {
        record["supplier_code"] = code.clone();
    }
    if let Some(name) = tranche {
        record["tranche_name"] = json!(name);
    }
    if let Some(props) = props {
        record["tranche_properties"] = props;
    }
    Some(record)
}

fn page(
    records: Vec<Value>,
    counts: BTreeMap<String, usize>,
    cap: usize,
    query: Value,
    missing: Vec<String>,
) -> Value {
    let total = records.len();
    let returned: Vec<Value> = records.into_iter().take(cap).collect();
    json!({
        "source": "ZINC CartBlanche22",
        "source_url": CARTBLANCHE,
        "query": query,
        "total_available": total,
        "returned": returned.len(),
        "truncated": total > returned.len(),
        "source_counts": counts,
        "missing_ids": missing,
        "records": returned,
    })
}

fn missing_zinc_ids(requested: &[String], records: &[Value]) -> Vec<String> {
    let found: HashSet<String> = records
        .iter()
        .filter_map(|record| record.get("zinc_id").and_then(Value::as_str))
        .map(canonical_zinc_id)
        .collect();
    requested
        .iter()
        .filter(|id| !found.contains(&canonical_zinc_id(id)))
        .cloned()
        .collect()
}

fn missing_supplier_codes(requested: &[String], records: &[Value]) -> Vec<String> {
    let mut found = HashSet::new();
    for record in records {
        for catalog in record
            .get("catalogs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(code) = catalog.get("supplier_code").and_then(Value::as_str) {
                found.insert(code);
            }
        }
        match record.get("supplier_code") {
            Some(Value::String(code)) => {
                found.insert(code.as_str());
            }
            Some(Value::Array(codes)) => {
                for code in codes {
                    if let Some(text) = code.as_str() {
                        found.insert(text);
                    }
                }
            }
            _ => {}
        }
    }
    requested
        .iter()
        .filter(|code| !found.contains(code.as_str()))
        .cloned()
        .collect()
}

fn require_ids(ids: &[String], bound: usize, what: &str, zinc_shaped: bool) -> Result<Vec<String>> {
    let mut cleaned = Vec::new();
    for id in ids {
        let entry = id.trim();
        if entry.is_empty() {
            continue;
        }
        if entry.chars().any(|c| c == ',' || c.is_whitespace()) {
            bail!(
                "{what} {entry:?} contains a comma or whitespace; pass each identifier as its own list item (at most {bound} per call)"
            );
        }
        if entry.len() > 128 {
            bail!("{what} exceeds 128 characters");
        }
        if zinc_shaped && !is_zinc_id(entry) {
            bail!(
                "{what} {entry:?} is not a ZINC identifier (ZINC followed by digits or a ZINC-22 base-62 code)"
            );
        }
        cleaned.push(entry.to_string());
    }
    if cleaned.is_empty() {
        bail!("provide at least one {what}");
    }
    if cleaned.len() > bound {
        bail!(
            "{} {what}s exceeds the per-call bound of {bound}",
            cleaned.len()
        );
    }
    Ok(cleaned)
}

fn is_zinc_id(value: &str) -> bool {
    value.len() >= 5
        && value.len() <= 24
        && value
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("ZINC"))
        && value
            .get(4..)
            .is_some_and(|rest| rest.bytes().all(|b| b.is_ascii_alphanumeric()))
}

fn canonical_zinc_id(value: &str) -> String {
    let rest = value.get(4..).unwrap_or(value);
    if rest.bytes().all(|b| b.is_ascii_digit()) {
        format!("ZINC{rest:0>12}")
    } else if value.len() >= 4 && value[..4].eq_ignore_ascii_case("ZINC") {
        format!("ZINC{rest}")
    } else {
        value.to_string()
    }
}

fn bound_page(n: u32) -> Result<usize> {
    if !(1..=MAX_RESULTS).contains(&n) {
        bail!("max_results must be between 1 and {MAX_RESULTS}");
    }
    Ok(n as usize)
}

fn clamp_timeout(value: f64) -> Result<f64> {
    if !value.is_finite() {
        bail!("timeout_s must be a finite number of seconds");
    }
    Ok(value.clamp(MIN_TIMEOUT, MAX_TIMEOUT))
}

fn api_base(bio: &NativeBio) -> String {
    bio.credential("ZINC_BASE_URL")
        .map(|value| value.trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| CARTBLANCHE.to_string())
}

fn compound_url(zinc_id: &str) -> String {
    format!("{CARTBLANCHE}/substance/{}", path_segment(zinc_id))
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

fn record_string(record: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        match record.get(*key) {
            Some(Value::String(text)) if !text.is_empty() => return Some(text.clone()),
            Some(Value::Number(number)) => return Some(number.to_string()),
            _ => {}
        }
    }
    None
}

fn tranche_code(record: &Value) -> Option<String> {
    if let Some(name) = record_string(record, &["tranche_name"]) {
        return parse_tranche(&name).map(|(_, _, code)| code);
    }
    match record.get("tranche") {
        Some(Value::String(text)) => parse_tranche(text).map(|(_, _, code)| code),
        Some(Value::Object(map)) => {
            let h = map.get("h_num")?.as_str()?;
            let p = map.get("p_num")?.as_str()?;
            parse_tranche(&format!("{h}{p}")).map(|(_, _, code)| code)
        }
        _ => None,
    }
}

fn parse_tranche(code: &str) -> Option<(u32, f64, String)> {
    let code = code.trim();
    let bytes = code.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'H' {
        return None;
    }
    if !bytes[1].is_ascii_digit() || !bytes[2].is_ascii_digit() {
        return None;
    }
    let sign = match bytes[3] {
        b'P' => 1.0,
        b'M' => -1.0,
        _ => return None,
    };
    if !bytes[4].is_ascii_digit() || !bytes[5].is_ascii_digit() || !bytes[6].is_ascii_digit() {
        return None;
    }
    let heavy_atoms = code[1..3].parse().ok()?;
    let bin: u32 = code[4..7].parse().ok()?;
    Some((heavy_atoms, sign * f64::from(bin) / 100.0, code.to_string()))
}

fn tranche_details(value: &Value) -> Option<Value> {
    let details = value.as_object()?;
    if !details.contains_key("heavy_atoms") {
        return None;
    }
    let mut props = json!({
        "heavy_atoms": details.get("heavy_atoms"),
        "logp": details.get("logp"),
    });
    if details.get("mwt").is_some() {
        props["mwt"] = details.get("mwt").cloned()?;
    }
    Some(props)
}
