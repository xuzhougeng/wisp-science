use super::{cadd_base, json_i64, json_string, NativeBio};
use crate::http::Source;
use anyhow::{bail, Context, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::time::Duration;

const CADD: Source = Source("CADD", Duration::from_millis(500));
const CADD_DOCS: &str = "https://cadd.gs.washington.edu/api";
const DEFAULT_VERSION: &str = "GRCh38-v1.7";
const MAX_RANGE_BP: i64 = 100;
const RANGE_HEADER: [&str; 6] = ["Chrom", "Pos", "Ref", "Alt", "RawScore", "PHRED"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PositionScores {
    chrom: String,
    pos: i64,
    #[serde(default = "default_version")]
    version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VariantScore {
    chrom: String,
    pos: i64,
    #[serde(rename = "ref")]
    reference: String,
    alt: String,
    #[serde(default = "default_version")]
    version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RangeScores {
    chrom: String,
    start: i64,
    end: i64,
    #[serde(default = "default_version")]
    version: String,
}

fn default_version() -> String {
    DEFAULT_VERSION.into()
}

pub(super) async fn position_scores(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: PositionScores =
        serde_json::from_value(args.clone()).context("invalid cadd_position_scores arguments")?;
    let version = require_version(&args.version)?;
    let chrom = require_chrom(&args.chrom)?;
    let pos = require_pos(args.pos, "pos")?;
    let records = fetch_position(bio, &version, &chrom, pos).await?;
    Ok(json!({
        "source": "CADD",
        "source_url": CADD_DOCS,
        "query": {
            "type": "position",
            "version": version,
            "chrom": chrom,
            "pos": pos
        },
        "records": records
    }))
}

pub(super) async fn variant_score(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: VariantScore =
        serde_json::from_value(args.clone()).context("invalid cadd_variant_score arguments")?;
    let version = require_version(&args.version)?;
    let chrom = require_chrom(&args.chrom)?;
    let pos = require_pos(args.pos, "pos")?;
    let reference = require_allele(&args.reference, "ref")?;
    let alt = require_allele(&args.alt, "alt")?;
    if reference == alt {
        bail!("ref and alt must differ");
    }
    let records = fetch_position(bio, &version, &chrom, pos).await?;
    let actual_ref = records[0]["ref"].as_str().unwrap_or("");
    if !actual_ref.eq_ignore_ascii_case(&reference) {
        bail!(
            "CADD {version} {chrom}:{pos}: query ref={reference} but the reference allele is {actual_ref} (wrong build or typo)"
        );
    }
    if let Some(record) = records.iter().find(|row| {
        row["alt"]
            .as_str()
            .is_some_and(|value| value.eq_ignore_ascii_case(&alt))
    }) {
        return Ok(json!({
            "source": "CADD",
            "source_url": CADD_DOCS,
            "query": {
                "type": "variant",
                "version": version,
                "chrom": chrom,
                "pos": pos,
                "ref": reference,
                "alt": alt
            },
            "record": record
        }));
    }
    let alts: Vec<&str> = records
        .iter()
        .filter_map(|row| row["alt"].as_str())
        .collect();
    bail!(
        "no CADD row for alt={alt} at {version} {chrom}:{pos} (alts present: {})",
        alts.join(", ")
    )
}

pub(super) async fn range_scores(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: RangeScores =
        serde_json::from_value(args.clone()).context("invalid cadd_range_scores arguments")?;
    let version = require_version(&args.version)?;
    let chrom = require_chrom(&args.chrom)?;
    let (start, end, span) = require_span(args.start, args.end)?;
    let path = format!("{version}/{chrom}:{start}-{end}");
    let payload = cadd_get(bio, &path).await?;
    let records = parse_range(&payload)?;
    if records.is_empty() {
        bail!("no CADD rows for {version} {chrom}:{start}-{end}");
    }
    let positions: BTreeSet<i64> = records
        .iter()
        .filter_map(|row| json_i64(&row["pos"]))
        .collect();
    Ok(json!({
        "source": "CADD",
        "source_url": CADD_DOCS,
        "query": {
            "type": "range",
            "version": version,
            "chrom": chrom,
            "start": start,
            "end": end
        },
        "n_records": records.len(),
        "n_positions_scored": positions.len(),
        "span_bp": span,
        "truncated": false,
        "records": records
    }))
}

async fn fetch_position(
    bio: &NativeBio,
    version: &str,
    chrom: &str,
    pos: i64,
) -> Result<Vec<Value>> {
    let path = format!("{version}/{chrom}:{pos}");
    let payload = cadd_get(bio, &path).await?;
    let records = parse_position(&payload)?;
    if records.is_empty() {
        bail!("no CADD rows for {version} {chrom}:{pos}");
    }
    Ok(records)
}

async fn cadd_get(bio: &NativeBio, path: &str) -> Result<Value> {
    let url = format!("{}/{path}", cadd_base(bio));
    bio.http().send(CADD, Method::GET, &url, &[]).await?.json()
}

fn parse_position(payload: &Value) -> Result<Vec<Value>> {
    let rows = payload
        .as_array()
        .context("CADD position response must be a JSON list")?;
    let mut records = Vec::with_capacity(rows.len());
    for row in rows {
        if !row.is_object() {
            bail!("CADD position row is missing required keys");
        }
        records.push(score_record(
            &row["Chrom"],
            &row["Pos"],
            &row["Ref"],
            &row["Alt"],
            &row["RawScore"],
            &row["PHRED"],
        )?);
    }
    sort_records(&mut records);
    Ok(records)
}

fn parse_range(payload: &Value) -> Result<Vec<Value>> {
    let rows = payload
        .as_array()
        .context("CADD range response must be a JSON list")?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let header = rows[0]
        .as_array()
        .context("CADD range response had an unexpected header")?;
    if header.len() != RANGE_HEADER.len()
        || header
            .iter()
            .zip(RANGE_HEADER)
            .any(|(got, want)| got.as_str() != Some(want))
    {
        bail!("CADD range response had an unexpected header");
    }
    let mut records = Vec::with_capacity(rows.len().saturating_sub(1));
    for row in &rows[1..] {
        let cells = row
            .as_array()
            .filter(|cells| cells.len() == RANGE_HEADER.len())
            .context("CADD range row is missing required keys")?;
        records.push(score_record(
            &cells[0], &cells[1], &cells[2], &cells[3], &cells[4], &cells[5],
        )?);
    }
    sort_records(&mut records);
    Ok(records)
}

fn score_record(
    chrom: &Value,
    pos: &Value,
    reference: &Value,
    alt: &Value,
    raw_score: &Value,
    phred: &Value,
) -> Result<Value> {
    let chrom = json_string(chrom).context("CADD omitted Chrom")?;
    let pos = json_i64(pos)
        .filter(|value| *value >= 1)
        .context("CADD omitted Pos")?;
    let reference = json_string(reference).context("CADD omitted Ref")?;
    let alt = json_string(alt).context("CADD omitted Alt")?;
    let raw_score = decimal_string(raw_score, "RawScore")?;
    let phred = decimal_string(phred, "PHRED")?;
    Ok(json!({
        "chrom": chrom,
        "pos": pos,
        "ref": reference,
        "alt": alt,
        "raw_score": raw_score,
        "phred": phred
    }))
}

fn decimal_string(value: &Value, what: &str) -> Result<String> {
    match value {
        Value::String(text) if !text.is_empty() => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        _ => bail!("CADD omitted {what}"),
    }
}

fn sort_records(records: &mut [Value]) {
    records.sort_by(|a, b| {
        a["chrom"]
            .as_str()
            .unwrap_or("")
            .cmp(b["chrom"].as_str().unwrap_or(""))
            .then_with(|| {
                json_i64(&a["pos"])
                    .unwrap_or(0)
                    .cmp(&json_i64(&b["pos"]).unwrap_or(0))
            })
            .then_with(|| {
                a["ref"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["ref"].as_str().unwrap_or(""))
            })
            .then_with(|| {
                a["alt"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["alt"].as_str().unwrap_or(""))
            })
    });
}

pub(super) fn require_version(raw: &str) -> Result<String> {
    let text = raw.trim();
    let Some(rest) = text
        .strip_prefix("GRCh37-v")
        .or_else(|| text.strip_prefix("GRCh38-v"))
    else {
        bail!(
            "CADD version must look like GRCh38-v1.7 (build prefix required; a bare v1.7 is rejected)"
        );
    };
    let numeric = match rest.strip_suffix("_inclAnno") {
        Some(numeric) => numeric,
        None if rest.contains("_inclAnno") => {
            bail!(
                "CADD version must look like GRCh38-v1.7 (build prefix required; a bare v1.7 is rejected)"
            );
        }
        None => rest,
    };
    let mut parts = numeric.split('.');
    let major = parts.next().unwrap_or("");
    let minor = parts.next().unwrap_or("");
    if parts.next().is_some()
        || major.is_empty()
        || minor.is_empty()
        || !major.bytes().all(|b| b.is_ascii_digit())
        || !minor.bytes().all(|b| b.is_ascii_digit())
    {
        bail!(
            "CADD version must look like GRCh38-v1.7 (build prefix required; a bare v1.7 is rejected)"
        );
    }
    Ok(text.to_string())
}

pub(super) fn require_chrom(raw: &str) -> Result<String> {
    let text = raw.trim();
    let text = match text.get(..3) {
        Some(prefix) if prefix.eq_ignore_ascii_case("chr") => &text[3..],
        _ => text,
    };
    if text.is_empty() || text.len() > 2 {
        bail!("CADD chromosome must be 1–22, X or Y (nuclear SNVs only)");
    }
    let upper = text.to_ascii_uppercase();
    if let Ok(n) = upper.parse::<u8>() {
        if (1..=22).contains(&n) {
            return Ok(n.to_string());
        }
    } else if upper == "X" || upper == "Y" {
        return Ok(upper);
    }
    if matches!(upper.as_str(), "M" | "MT") {
        bail!("CADD scores nuclear SNVs only; mitochondrial contigs are rejected");
    }
    bail!("CADD chromosome must be 1–22, X or Y (nuclear SNVs only)")
}

fn require_pos(value: i64, what: &str) -> Result<i64> {
    if value < 1 {
        bail!("{what} must be a positive 1-based position");
    }
    Ok(value)
}

pub(super) fn require_allele(raw: &str, what: &str) -> Result<String> {
    let text = raw.trim().to_ascii_uppercase();
    if text.len() != 1 || !matches!(text.as_bytes()[0], b'A' | b'C' | b'G' | b'T') {
        bail!("{what} must be one of A/C/G/T");
    }
    Ok(text)
}

pub(super) fn require_span(start: i64, end: i64) -> Result<(i64, i64, i64)> {
    let start = require_pos(start, "start")?;
    let end = require_pos(end, "end")?;
    if end < start {
        bail!("end must be ≥ start");
    }
    let span = end
        .checked_sub(start)
        .and_then(|delta| delta.checked_add(1))
        .context("CADD range is too large")?;
    if span > MAX_RANGE_BP {
        bail!("CADD range spans {span} bp; maximum is {MAX_RANGE_BP} bp (split into consecutive windows)");
    }
    Ok((start, end, span))
}
