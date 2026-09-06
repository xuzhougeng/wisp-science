use super::{
    api_base, as_i64, bound_int, json_string, listify, object_field, path_segment, require_ok,
    require_text, send_json, text_field, unique_ids, COMPLEXPORTAL_DEFAULT, COMPLEXPORTAL_SITE,
    COMPLEX_PORTAL,
};
use crate::NativeBio;
use anyhow::{Context, Result};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};

const MAX_IDS: usize = 25;
const PAGE_SIZE: usize = 50;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetComplexes {
    complex_acs: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    accession: String,
    #[serde(default = "default_true")]
    participants_only: bool,
    #[serde(default = "default_max")]
    max_results: u32,
}

fn default_true() -> bool {
    true
}

fn default_max() -> u32 {
    50
}

pub(crate) fn fold_complex_ac(raw: &str) -> Result<Option<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let upper = trimmed.to_ascii_uppercase();
    if !upper.starts_with("CPX-")
        || upper.len() > 32
        || !upper[4..].bytes().all(|b| b.is_ascii_digit())
    {
        anyhow::bail!("Complex Portal accession {trimmed:?} must look like CPX-1234");
    }
    Ok(Some(upper))
}

pub(crate) async fn get_complexes(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetComplexes =
        serde_json::from_value(args.clone()).context("invalid Complex Portal lookup arguments")?;
    let ids = unique_ids(
        &args.complex_acs,
        MAX_IDS,
        "Complex Portal accession",
        fold_complex_ac,
    )?;
    let mut records = Vec::new();
    let mut not_found = Vec::new();
    for ac in &ids.unique {
        match fetch_complex(bio, ac).await? {
            None => not_found.push(ac.clone()),
            Some(raw) => records.push(parse_complex(&raw)),
        }
    }
    Ok(json!({
        "source": "Complex Portal",
        "source_url": COMPLEXPORTAL_SITE,
        "n_requested": ids.requested,
        "n_unique": ids.unique.len(),
        "n_blank_skipped": ids.n_blank,
        "n_duplicate_skipped": ids.n_duplicate,
        "not_found": not_found,
        "records": records
    }))
}

pub(crate) async fn search_by_participant(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Search =
        serde_json::from_value(args.clone()).context("invalid Complex Portal search arguments")?;
    let accession = require_text(&args.accession, "accession", 64)?;
    let cap = bound_int(args.max_results, 1, 200, "max_results")?;
    let query = if args.participants_only {
        format!("pxref:\"{accession}\"")
    } else {
        accession.clone()
    };
    let base = api_base(bio, "COMPLEXPORTAL_URL", COMPLEXPORTAL_DEFAULT);
    let mut elements = Vec::new();
    let mut first = 0usize;
    let mut total_reported: Option<u64> = None;
    loop {
        let rows = PAGE_SIZE.min(cap.saturating_sub(elements.len()));
        if rows == 0 {
            break;
        }
        let url = format!("{base}/search/{}", path_segment(&query));
        let (status, body) = send_json(
            bio,
            COMPLEX_PORTAL,
            Method::GET,
            &url,
            &[
                ("format".into(), "json".into()),
                ("first".into(), first.to_string()),
                ("number".into(), rows.to_string()),
            ],
        )
        .await?;
        require_ok(COMPLEX_PORTAL, status)?;
        let body = body.unwrap_or(json!({}));
        let page_total = body
            .get("totalNumberOfResults")
            .and_then(|v| v.as_u64())
            .or_else(|| {
                body.get("totalNumberOfResults")
                    .and_then(as_i64)
                    .and_then(|n| u64::try_from(n).ok())
            });
        if let Some(total) = page_total {
            total_reported = Some(total);
        }
        let batch = body
            .get("elements")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let n = batch.len();
        elements.extend(batch);
        first += n;
        let total = total_reported.unwrap_or(first as u64);
        if n == 0 || first >= total as usize || elements.len() >= cap {
            break;
        }
    }
    let mut records: Vec<Value> = elements.iter().map(parse_search_element).collect();
    records.sort_by(|a, b| cpx_sort_key(a).cmp(&cpx_sort_key(b)));
    let total = total_reported.unwrap_or(records.len() as u64);
    Ok(json!({
        "source": "Complex Portal",
        "source_url": COMPLEXPORTAL_SITE,
        "query_accession": accession,
        "solr_query": query,
        "participants_only": args.participants_only,
        "total_reported": total,
        "returned": records.len(),
        "truncated": total > records.len() as u64,
        "complexes": records
    }))
}

async fn fetch_complex(bio: &NativeBio, ac: &str) -> Result<Option<Value>> {
    let base = api_base(bio, "COMPLEXPORTAL_URL", COMPLEXPORTAL_DEFAULT);
    let url = format!("{base}/complex/{}", path_segment(ac));
    let (status, body) = send_json(bio, COMPLEX_PORTAL, Method::GET, &url, &[]).await?;
    if status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    require_ok(COMPLEX_PORTAL, status)?;
    Ok(Some(body.context("Complex Portal returned an empty body")?))
}

fn parse_complex(raw: &Value) -> Value {
    let (species_name, taxid) = split_species(text_field(raw, &["species"]));
    let mut participants: Vec<Value> = listify(raw.get("participants"))
        .into_iter()
        .map(parse_participant)
        .collect();
    participants.sort_by(|a, b| {
        (
            a.get("interactor_type")
                .and_then(Value::as_str)
                .unwrap_or(""),
            a.get("identifier").and_then(Value::as_str).unwrap_or(""),
        )
            .cmp(&(
                b.get("interactor_type")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                b.get("identifier").and_then(Value::as_str).unwrap_or(""),
            ))
    });
    let mut go = Vec::new();
    let mut xrefs = Vec::new();
    for xref in listify(raw.get("crossReferences")) {
        let db = text_field(xref, &["database"]).unwrap_or_default();
        if db.eq_ignore_ascii_case("gene ontology") {
            go.push(json!({
                "go_id": json_string(xref.get("identifier")),
                "aspect": json_string(xref.get("qualifier")),
                "term": json_string(xref.get("description"))
            }));
        } else {
            xrefs.push(json!({
                "database": db,
                "identifier": json_string(xref.get("identifier")),
                "qualifier": json_string(xref.get("qualifier")),
                "description": json_string(xref.get("description"))
            }));
        }
    }
    let evidence = object_field(raw, "evidenceType")
        .map(|map| {
            json!({
                "eco_code": json_string(map.get("identifier")),
                "description": json_string(map.get("description")),
                "confidence_score": map.get("confidenceScore").cloned().unwrap_or(Value::Null)
            })
        })
        .unwrap_or(Value::Null);
    let ac = text_field(raw, &["complexAc"]).unwrap_or_default();
    json!({
        "complex_ac": ac,
        "url": complex_url(&ac),
        "intact_ac": json_string(raw.get("ac")),
        "name": json_string(raw.get("name")),
        "systematic_name": json_string(raw.get("systematicName")),
        "synonyms": string_sorted(raw.get("synonyms")),
        "species_name": json!(species_name),
        "taxid": json!(taxid),
        "predicted_complex": raw.get("predictedComplex").cloned().unwrap_or(Value::Null),
        "evidence": evidence,
        "participants": participants,
        "go_annotations": go,
        "cross_references": xrefs,
        "functions": raw.get("functions").cloned().unwrap_or_else(|| json!([])),
        "complex_assemblies": raw.get("complexAssemblies").cloned().unwrap_or_else(|| json!([])),
        "release_dates": raw.get("releaseDates").cloned().unwrap_or_else(|| json!([]))
    })
}

fn parse_participant(raw: &Value) -> Value {
    let (smin, smax) = parse_stoichiometry(text_field(raw, &["stochiometry", "stoichiometry"]));
    json!({
        "identifier": json_string(raw.get("identifier")),
        "name": json_string(raw.get("name")),
        "description": json_string(raw.get("description")),
        "interactor_type": json_string(raw.get("interactorType")),
        "interactor_type_mi": json_string(raw.get("interactorTypeMI")),
        "biological_role": json_string(raw.get("bioRole")),
        "biological_role_mi": json_string(raw.get("bioRoleMI")),
        "stoichiometry_min": json!(smin),
        "stoichiometry_max": json!(smax),
        "stoichiometry_raw": json_string(raw.get("stochiometry").or(raw.get("stoichiometry")))
    })
}

fn parse_search_element(raw: &Value) -> Value {
    let (species_name, taxid) = split_species(text_field(raw, &["organismName"]));
    let ac = text_field(raw, &["complexAC", "complexAc"]).unwrap_or_default();
    json!({
        "complex_ac": ac,
        "url": complex_url(&ac),
        "name": json_string(raw.get("complexName")),
        "species_name": json!(species_name),
        "taxid": json!(taxid),
        "predicted_complex": raw.get("predictedComplex").cloned().unwrap_or(Value::Null)
    })
}

fn parse_stoichiometry(raw: Option<String>) -> (Option<i64>, Option<i64>) {
    let Some(raw) = raw else {
        return (None, None);
    };
    let lower = raw.to_ascii_lowercase();
    let min = capture_after(&lower, "minvalue:");
    let max = capture_after(&lower, "maxvalue:");
    (min, max)
}

fn capture_after(haystack: &str, prefix: &str) -> Option<i64> {
    let rest = haystack.split(prefix).nth(1)?;
    rest.chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

fn split_species(raw: Option<String>) -> (Option<String>, Option<i64>) {
    let Some(raw) = raw else {
        return (None, None);
    };
    if let Some((name, tax)) = raw.rsplit_once(';') {
        (Some(name.trim().to_string()), tax.trim().parse().ok())
    } else {
        (Some(raw), None)
    }
}

fn string_sorted(node: Option<&Value>) -> Value {
    let mut values: Vec<String> = listify(node)
        .into_iter()
        .filter_map(|item| item.as_str().map(str::to_string))
        .collect();
    values.sort();
    json!(values)
}

fn cpx_sort_key(row: &Value) -> (u8, u64, String) {
    let ac = row
        .get("complex_ac")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match ac.strip_prefix("CPX-").and_then(|n| n.parse::<u64>().ok()) {
        Some(n) => (0, n, ac.to_string()),
        None => (1, 0, ac.to_string()),
    }
}

fn complex_url(ac: &str) -> String {
    format!("{COMPLEXPORTAL_SITE}/complex/{}", path_segment(ac))
}
