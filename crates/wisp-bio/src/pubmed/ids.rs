//! NCBI PMC ID Converter. Official reference (reviewed 2026-09-06):
//! https://pmc.ncbi.nlm.nih.gov/tools/id-converter-api/
use super::{json_flag, json_id, records, PubMed};
use crate::http::PMC_IDCONV;
use anyhow::{bail, Context, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};

pub(super) const SOURCE_URL: &str = "https://pmc.ncbi.nlm.nih.gov/tools/idconv/api/v1/articles/";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Convert {
    ids: Vec<String>,
    #[serde(default = "default_id_type")]
    id_type: String,
}

fn default_id_type() -> String {
    "pmid".into()
}

pub(super) async fn convert(client: &PubMed, args: &Value) -> Result<Value> {
    let args: Convert =
        serde_json::from_value(args.clone()).context("invalid identifier conversion arguments")?;
    let (id_type, ids) = convert_params(&args)?;
    let mut params = vec![
        ("ids".into(), ids.join(",")),
        ("idtype".into(), id_type.clone()),
        ("format".into(), "json".into()),
        ("tool".into(), "wisp-science".into()),
    ];
    if let Some(email) = &client.email {
        params.push(("email".into(), email.clone()));
    }
    let raw = client
        .http
        .send(PMC_IDCONV, Method::GET, &client.idconv, &params)
        .await?
        .json()?;
    if raw.get("status").and_then(Value::as_str) == Some("error") || raw.get("error").is_some() {
        bail!("PMC ID Converter rejected the request");
    }
    convert_result(&raw, &ids, &id_type)
}

fn convert_params(args: &Convert) -> Result<(String, Vec<String>)> {
    let id_type = match args.id_type.as_str() {
        "pmid" | "pmcid" | "doi" => args.id_type.clone(),
        _ => bail!("id_type must be pmid, pmcid or doi"),
    };
    if !(1..=200).contains(&args.ids.len()) {
        bail!("provide 1 to 200 identifiers of a single type");
    }
    let ids = args
        .ids
        .iter()
        .map(|id| normalize_id(id, &id_type))
        .collect::<Result<Vec<_>>>()?;
    Ok((id_type, ids))
}

fn normalize_id(id: &str, id_type: &str) -> Result<String> {
    let id = id.trim();
    match id_type {
        "pmid" => super::require_pmid(id).map(str::to_string),
        "pmcid" => records::pmc_id(id)
            .ok_or_else(|| anyhow::anyhow!("PMCID values must be PMC followed by digits")),
        "doi" => {
            if id.len() <= 256
                && id.starts_with("10.")
                && id.contains('/')
                && !id.chars().any(char::is_whitespace)
            {
                Ok(id.to_string())
            } else {
                bail!("DOI values must start with 10. and contain a slash");
            }
        }
        _ => bail!("id_type must be pmid, pmcid or doi"),
    }
}

pub(super) fn convert_result(raw: &Value, requested: &[String], id_type: &str) -> Result<Value> {
    let records_in = raw
        .get("records")
        .and_then(Value::as_array)
        .context("PMC ID Converter omitted records")?;
    let mut by_id = std::collections::HashMap::new();
    for record in records_in {
        if let Some(key) =
            json_id(&record["requested-id"]).or_else(|| json_id(&record["requested_id"]))
        {
            by_id
                .entry(normalize_match(&key, id_type))
                .or_insert(record);
        }
    }
    let mut records = Vec::new();
    let mut missing = Vec::new();
    let mut unconverted = Vec::new();
    for id in requested {
        let Some(record) = by_id.get(id).copied() else {
            missing.push(id.clone());
            continue;
        };
        if record.get("status").and_then(Value::as_str) == Some("error")
            || record.get("errmsg").is_some()
            || record.get("error").is_some()
        {
            unconverted.push(json!({
                "requested_id": id,
                "reason": record.get("errmsg").and_then(json_id)
                    .or_else(|| record.get("error").and_then(json_id))
                    .unwrap_or_else(|| "converter could not map this identifier".into())
            }));
            continue;
        }
        let pmid = json_id(&record["pmid"]);
        let pmcid = json_id(&record["pmcid"]).and_then(|value| records::pmc_id(&value));
        let doi = json_id(&record["doi"]);
        if pmcid.is_none() {
            unconverted.push(json!({
                "requested_id": id,
                "pmid": pmid,
                "doi": doi,
                "reason": "article is not in PubMed Central; the converter only maps PMC holdings"
            }));
            continue;
        }
        let pmcid = pmcid.unwrap();
        let live = json_flag(&record["live"]);
        let release_date =
            json_id(&record["release-date"]).or_else(|| json_id(&record["release_date"]));
        records.push(json!({
            "requested_id": id,
            "pmid": pmid,
            "pmcid": pmcid,
            "doi": doi,
            "live": live,
            "release_date": release_date,
            "in_pmc": true,
            "url": format!("https://www.ncbi.nlm.nih.gov/pmc/articles/{pmcid}/")
        }));
    }
    Ok(json!({
        "source": "NCBI PMC ID Converter",
        "source_url": SOURCE_URL,
        "id_type": id_type,
        "requested": requested,
        "returned": records.len(),
        "records": records,
        "missing_ids": missing,
        "unconverted_ids": unconverted
    }))
}

fn normalize_match(value: &str, id_type: &str) -> String {
    match id_type {
        "pmcid" => records::pmc_id(value).unwrap_or_else(|| value.to_string()),
        _ => value.to_string(),
    }
}

#[cfg(test)]
pub(super) fn parse_args(args: Value) -> Result<(String, Vec<String>)> {
    let args: Convert = serde_json::from_value(args)?;
    convert_params(&args)
}
