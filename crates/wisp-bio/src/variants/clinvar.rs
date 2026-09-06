use super::{
    clinvar_url, json_i64, json_string, ncbi_json, page_bound, require_rsid, require_text,
    NativeBio, BATCH_DEADLINE, CLINVAR_BROWSER,
};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Instant;

const MAX_ACCESSIONS: usize = 50;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Search {
    query: String,
    #[serde(default = "default_page")]
    max_records: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Records {
    accessions: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ByRsid {
    rsid: String,
    #[serde(default = "default_page")]
    max_records: u32,
}

fn default_page() -> u32 {
    50
}

pub(super) async fn search(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Search =
        serde_json::from_value(args.clone()).context("invalid clinvar_search arguments")?;
    let term = require_text(&args.query, 1, 8192, "query")?;
    let retmax = page_bound(args.max_records, 1, 200, "max_records")?;
    let (total, uids) = esearch(bio, &term, retmax).await?;
    let (records, missing) = summaries(bio, &uids, None).await?;
    Ok(json!({
        "source": "NCBI ClinVar",
        "source_url": CLINVAR_BROWSER,
        "term": term,
        "total": total,
        "n_returned": records.len(),
        "truncated": total > uids.len() as u64,
        "missing_uids": missing,
        "records": records
    }))
}

pub(super) async fn get_records(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Records =
        serde_json::from_value(args.clone()).context("invalid clinvar_get_records arguments")?;
    let mut cleaned = Vec::new();
    for raw in &args.accessions {
        let item = raw.trim();
        if item.is_empty() {
            continue;
        }
        if item.len() > 32 {
            bail!("ClinVar accession exceeds 32 characters");
        }
        cleaned.push(item.to_string());
    }
    if cleaned.is_empty() {
        bail!("provide 1 to {MAX_ACCESSIONS} ClinVar accessions");
    }
    let mut pending = Vec::new();
    for item in &cleaned {
        if !pending
            .iter()
            .any(|seen: &String| seen.eq_ignore_ascii_case(item))
        {
            pending.push(item.clone());
        }
    }
    if pending.len() > MAX_ACCESSIONS {
        bail!("at most {MAX_ACCESSIONS} unique ClinVar accessions per call");
    }
    let mut uid_sources: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut rcvs = Vec::new();
    for acc in &pending {
        if let Some(uid) = vcv_uid(acc) {
            uid_sources.entry(uid).or_default().push(acc.clone());
        } else if acc.bytes().all(|b| b.is_ascii_digit()) {
            let uid = acc.trim_start_matches('0');
            if uid.is_empty() {
                bail!("{acc:?} is not a ClinVar variation ID");
            }
            uid_sources
                .entry(uid.to_string())
                .or_default()
                .push(acc.clone());
        } else if is_rcv(acc) {
            rcvs.push(acc.clone());
        } else if require_rsid(acc).is_ok() {
            bail!("{acc} is an rsID — use clinvar_variant_by_rsid");
        } else {
            bail!("unrecognized ClinVar accession {acc:?} (expected VCVnnn, RCVnnn, or a variation ID)");
        }
    }
    let mut not_found = Vec::new();
    let mut not_processed = Vec::new();
    let started = Instant::now();
    for (index, acc) in rcvs.iter().enumerate() {
        if started.elapsed() > BATCH_DEADLINE {
            not_processed.extend(rcvs[index..].iter().cloned());
            break;
        }
        let term = acc.split('.').next().unwrap_or(acc).to_ascii_uppercase();
        let (_total, uids) = esearch(bio, &term, 5).await?;
        if uids.is_empty() {
            not_found.push(acc.clone());
        }
        for uid in uids {
            uid_sources.entry(uid).or_default().push(acc.clone());
        }
    }
    let uids: Vec<String> = uid_sources.keys().cloned().collect();
    let (mut records, missing) = summaries(bio, &uids, Some(&uid_sources)).await?;
    records.sort_by_key(|row| json_i64(&row["variation_id"]).unwrap_or(0));
    Ok(json!({
        "source": "NCBI ClinVar",
        "source_url": CLINVAR_BROWSER,
        "n_requested": cleaned.len(),
        "n_unique": pending.len(),
        "records": records,
        "not_found": not_found,
        "missing_uids": missing,
        "not_processed": not_processed
    }))
}

pub(super) async fn by_rsid(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ByRsid = serde_json::from_value(args.clone())
        .context("invalid clinvar_variant_by_rsid arguments")?;
    let (_number, rsid) = require_rsid(&args.rsid)?;
    let retmax = page_bound(args.max_records, 1, 200, "max_records")?;
    let (total, uids) = esearch(bio, &rsid, retmax).await?;
    let (records, missing) = summaries(bio, &uids, None).await?;
    Ok(json!({
        "source": "NCBI ClinVar",
        "source_url": CLINVAR_BROWSER,
        "rsid": rsid,
        "total": total,
        "n_returned": records.len(),
        "truncated": total > uids.len() as u64,
        "missing_uids": missing,
        "records": records
    }))
}

async fn esearch(bio: &NativeBio, term: &str, retmax: usize) -> Result<(u64, Vec<String>)> {
    let raw = ncbi_json(
        bio,
        "esearch.fcgi",
        vec![
            ("db".into(), "clinvar".into()),
            ("term".into(), term.to_string()),
            ("retmax".into(), retmax.to_string()),
            ("retstart".into(), "0".into()),
        ],
    )
    .await?;
    let result = raw
        .get("esearchresult")
        .context("ClinVar omitted search results")?;
    if result.get("ERROR").is_some() {
        bail!("ClinVar rejected the search expression");
    }
    let total = result
        .get("count")
        .and_then(json_string)
        .and_then(|text| text.parse().ok())
        .context("ClinVar omitted the search count")?;
    let uids: Vec<String> = result
        .get("idlist")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(json_string)
        .collect();
    if uids.len() > retmax {
        bail!("ClinVar returned inconsistent pagination");
    }
    Ok((total, uids))
}

async fn summaries(
    bio: &NativeBio,
    uids: &[String],
    sources: Option<&BTreeMap<String, Vec<String>>>,
) -> Result<(Vec<Value>, Vec<String>)> {
    if uids.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let raw = ncbi_json(
        bio,
        "esummary.fcgi",
        vec![
            ("db".into(), "clinvar".into()),
            ("id".into(), uids.join(",")),
        ],
    )
    .await?;
    let result = raw
        .get("result")
        .and_then(Value::as_object)
        .context("ClinVar omitted variation summaries")?;
    let mut records = Vec::new();
    let mut missing = Vec::new();
    for uid in uids {
        match result
            .get(uid)
            .filter(|doc| doc.is_object() && doc.get("error").is_none())
        {
            Some(doc) => {
                let mut record = parse_summary(doc)?;
                if let Some(sources) = sources {
                    record["requested_as"] = json!(sources.get(uid).cloned().unwrap_or_default());
                }
                records.push(record);
            }
            None => {
                if let Some(sources) = sources {
                    missing.extend(
                        sources
                            .get(uid)
                            .cloned()
                            .unwrap_or_else(|| vec![uid.clone()]),
                    );
                } else {
                    missing.push(uid.clone());
                }
            }
        }
    }
    Ok((records, missing))
}

pub(super) fn parse_summary(doc: &Value) -> Result<Value> {
    let uid = json_string(&doc["uid"]).context("ClinVar summary omitted uid")?;
    let variation_set = doc
        .get("variation_set")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let vs0 = variation_set.first().cloned().unwrap_or(json!({}));
    let xrefs = vs0
        .get("variation_xrefs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut rsids = Vec::new();
    let mut other_xrefs = Vec::new();
    for xref in &xrefs {
        let db = json_string(&xref["db_source"]).unwrap_or_default();
        let id = json_string(&xref["db_id"]).unwrap_or_default();
        if db.eq_ignore_ascii_case("dbSNP") && !id.is_empty() {
            let digits = id.trim_start_matches("rs").trim_start_matches("RS");
            rsids.push(format!("rs{digits}"));
        } else if !db.is_empty() {
            other_xrefs.push(json!({"db": db, "id": id}));
        }
    }
    let submissions = doc
        .get("supporting_submissions")
        .cloned()
        .unwrap_or(json!({}));
    let scv = submissions
        .get("scv")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let rcv = submissions
        .get("rcv")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let freqs: Vec<Value> = vs0
        .get("allele_freq_set")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|row| {
            json!({
                "source": row.get("source"),
                "minor_allele": row.get("minor_allele"),
                "value": row.get("value")
            })
        })
        .collect();
    let genes: Vec<Value> = doc
        .get("genes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|gene| {
            json!({
                "symbol": gene.get("symbol"),
                "gene_id": gene.get("geneid"),
                "strand": gene.get("strand")
            })
        })
        .collect();
    Ok(json!({
        "variation_id": uid.parse::<u64>().unwrap_or(0),
        "accession": doc.get("accession"),
        "accession_version": doc.get("accession_version"),
        "title": doc.get("title"),
        "obj_type": doc.get("obj_type"),
        "variant_type": vs0.get("variant_type"),
        "canonical_spdi": empty_to_null(vs0.get("canonical_spdi")),
        "cdna_change": empty_to_null(vs0.get("cdna_change")),
        "protein_change": empty_to_null(doc.get("protein_change")),
        "rsids": rsids,
        "other_xrefs": other_xrefs,
        "genes": genes,
        "molecular_consequences": doc.get("molecular_consequence_list").cloned().unwrap_or(json!([])),
        "locations": locations(&variation_set),
        "allele_frequencies": freqs,
        "germline_classification": classification(doc.get("germline_classification")),
        "clinical_impact_classification": classification(doc.get("clinical_impact_classification")),
        "oncogenicity_classification": classification(doc.get("oncogenicity_classification")),
        "n_submissions": scv.len(),
        "supporting_submissions": {"scv": scv, "rcv": rcv},
        "url": clinvar_url(&uid)
    }))
}

fn locations(variation_set: &[Value]) -> Vec<Value> {
    let mut locs = Vec::new();
    for vs in variation_set {
        for loc in vs
            .get("variation_loc")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            locs.push(json!({
                "status": loc.get("status"),
                "assembly": loc.get("assembly_name"),
                "chrom": loc.get("chr"),
                "band": empty_to_null(loc.get("band")),
                "start": json_i64(&loc["start"]),
                "stop": json_i64(&loc["stop"]),
                "ref": empty_to_null(loc.get("ref")),
                "alt": empty_to_null(loc.get("alt"))
            }));
        }
    }
    locs
}

fn classification(block: Option<&Value>) -> Value {
    let Some(block) = block.filter(|row| row.is_object()) else {
        return Value::Null;
    };
    let description = block
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let review = block
        .get("review_status")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if description.is_empty() && review.is_empty() {
        return Value::Null;
    }
    let mut conditions = Vec::new();
    for condition in block
        .get("trait_set")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = condition
            .get("trait_name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let xrefs: Vec<Value> = condition
            .get("trait_xrefs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|xref| json!({"db": xref.get("db_source"), "id": xref.get("db_id")}))
            .collect();
        if !name.is_empty() || !xrefs.is_empty() {
            conditions.push(json!({"name": name, "xrefs": xrefs}));
        }
    }
    json!({
        "description": description,
        "review_status": review,
        "gold_stars": gold_stars(review),
        "last_evaluated": classify_date(block.get("last_evaluated").and_then(Value::as_str)),
        "fda_recognized_database": empty_to_null(block.get("fda_recognized_database")),
        "conditions": conditions
    })
}

pub(super) fn gold_stars(review_status: &str) -> Value {
    match review_status.trim().to_ascii_lowercase().as_str() {
        "practice guideline" => json!(4),
        "reviewed by expert panel" => json!(3),
        "criteria provided, multiple submitters, no conflicts" => json!(2),
        "criteria provided, multiple submitters" => json!(2),
        "criteria provided, conflicting classifications"
        | "criteria provided, conflicting interpretations" => json!(1),
        "criteria provided, single submitter" => json!(1),
        "no assertion criteria provided"
        | "no classification provided"
        | "no classification for the individual variant"
        | "no classifications from unflagged records"
        | "no assertion provided" => json!(0),
        _ => Value::Null,
    }
}

fn classify_date(raw: Option<&str>) -> Value {
    let Some(raw) = raw.map(str::trim).filter(|text| !text.is_empty()) else {
        return Value::Null;
    };
    if raw == "1/01/01 00:00" {
        return Value::Null;
    }
    let date = raw.split(' ').next().unwrap_or(raw).replace('/', "-");
    json!(date)
}

fn empty_to_null(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) if text.trim().is_empty() => Value::Null,
        Some(value) if !value.is_null() => value.clone(),
        _ => Value::Null,
    }
}

fn vcv_uid(value: &str) -> Option<String> {
    let text = value.trim();
    let rest = text
        .strip_prefix("VCV")
        .or_else(|| text.strip_prefix("vcv"))?;
    let digits = rest.split('.').next().unwrap_or(rest);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(digits.trim_start_matches('0'))
        .filter(|uid| !uid.is_empty())
        .map(str::to_string)
}

fn is_rcv(value: &str) -> bool {
    let text = value.trim();
    let rest = match text
        .strip_prefix("RCV")
        .or_else(|| text.strip_prefix("rcv"))
    {
        Some(rest) => rest,
        None => return false,
    };
    let digits = rest.split('.').next().unwrap_or(rest);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}
