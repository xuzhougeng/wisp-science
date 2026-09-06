//! Europe PMC Articles REST. Official references (reviewed 2026-09-06):
//! - https://europepmc.org/RestfulWebService (core metadata and `{PMCID}/fullTextXML`)
//! - https://pmc.ncbi.nlm.nih.gov/tools/id-converter-api/ (embargo live / release-date)
//! - https://pmc.ncbi.nlm.nih.gov/tools/oa-service/ (retired; never called)
//!
//! `get_copyright_status` mapping away from the retired PMC OA Web Service:
//! - `copyright.statement` / `year` / `holder` were OA-service fields and are omitted.
//!   A stated license is Europe PMC core `license` when present.
//! - `license.type` / `url` from the OA service are not reproduced. The native
//!   `license.name` is the Europe PMC `license` string.
//! - Legacy `license.is_open_access` is `is_open_access` from Europe PMC
//!   `isOpenAccess`. That flag is access, not a reuse grant.
//! - `reuse_permission` is `unknown` unless a license string is present, in which
//!   case it is `license_stated`. An OA boolean never becomes `reuse_granted`.
//! - Metadata availability, accessible full text (`inEPMC`), and reuse are
//!   separate fields. Converter `live` / `release-date` are embargo attributes.
use super::{ids, json_flag, json_id, records, PubMed};
use crate::http::EUROPE_PMC;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};

pub(super) const SEARCH_URL: &str = "https://www.ebi.ac.uk/europepmc/webservices/rest/search";
pub(super) const REST_URL: &str = "https://www.ebi.ac.uk/europepmc/webservices/rest/";
pub(super) const CONTRACT_NOTE: &str = "Open-access metadata is not a reuse grant. Copyright and license fields come from Europe PMC core metadata and PMC ID Converter embargo attributes; the retired PMC OA Web Service is not used.";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FullText {
    pmc_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Copyright {
    pmids: Vec<String>,
}

pub(super) async fn full_text(client: &PubMed, args: &Value) -> Result<Value> {
    let args: FullText =
        serde_json::from_value(args.clone()).context("invalid full-text arguments")?;
    if !(1..=5).contains(&args.pmc_ids.len()) {
        bail!("provide 1 to 5 PMCIDs");
    }
    let pmc_ids = args
        .pmc_ids
        .iter()
        .map(|id| {
            records::pmc_id(id)
                .ok_or_else(|| anyhow!("PMCID values must be PMC followed by digits"))
        })
        .collect::<Result<Vec<_>>>()?;
    let query = pmc_ids
        .iter()
        .map(|id| format!("PMCID:{id}"))
        .collect::<Vec<_>>()
        .join(" OR ");
    let hits = epmc_hits(&search_core(client, &query, pmc_ids.len()).await?)?;
    let mut by_id = std::collections::HashMap::new();
    for hit in &hits {
        if let Some(id) = json_id(&hit["pmcid"]).and_then(|value| records::pmc_id(&value)) {
            by_id.insert(id, hit);
        }
    }
    let mut records_out = Vec::new();
    let mut not_found = Vec::new();
    let mut not_open_access = Vec::new();
    let mut xml_unavailable = Vec::new();
    for pmcid in &pmc_ids {
        let Some(hit) = by_id.get(pmcid) else {
            not_found.push(pmcid.clone());
            continue;
        };
        let pmid = json_id(&hit["pmid"]);
        let doi = json_id(&hit["doi"]);
        if json_flag(&hit["isOpenAccess"]) != Some(true) {
            not_open_access.push(json!({
                "pmcid": pmcid,
                "pmid": pmid,
                "doi": doi,
                "is_open_access": false,
                "in_europe_pmc": json_flag(&hit["inEPMC"]),
                "detail": "article is not in the Europe PMC open-access full-text subset"
            }));
            continue;
        }
        let url = format!("{pmcid}/fullTextXML");
        let response = client
            .http
            .send(
                EUROPE_PMC,
                Method::GET,
                &format!("{}{url}", client.europepmc),
                &[],
            )
            .await?;
        match response.status.as_u16() {
            200 => {
                let Ok(text) = String::from_utf8(response.body) else {
                    xml_unavailable.push(json!({
                        "pmcid": pmcid,
                        "detail": "full-text XML was not valid UTF-8"
                    }));
                    continue;
                };
                if text.trim().is_empty() {
                    xml_unavailable.push(json!({
                        "pmcid": pmcid,
                        "detail": "Europe PMC returned empty full-text XML"
                    }));
                    continue;
                }
                match records::full_text(&text, pmcid) {
                    Ok(extracted) => {
                        let mut record = extracted;
                        record["pmcid"] = json!(pmcid);
                        record["pmid"] = json!(pmid);
                        record["doi"] = json!(doi);
                        record["availability"] = json!("open_access_xml");
                        record["url"] = json!(format!(
                            "https://www.ebi.ac.uk/europepmc/webservices/rest/{pmcid}/fullTextXML"
                        ));
                        record["europe_pmc_url"] = json!(format!(
                            "https://europepmc.org/article/PMC/{}",
                            pmcid.trim_start_matches("PMC")
                        ));
                        records_out.push(record);
                    }
                    Err(error) => xml_unavailable.push(json!({
                        "pmcid": pmcid,
                        "detail": error.to_string()
                    })),
                }
            }
            404 => xml_unavailable.push(json!({
                "pmcid": pmcid,
                "pmid": pmid,
                "detail": "open-access metadata is present but full-text XML is unavailable"
            })),
            other => bail!("Europe PMC returned HTTP {other}"),
        }
    }
    Ok(json!({
        "source": "Europe PMC",
        "source_url": REST_URL,
        "requested": pmc_ids,
        "returned": records_out.len(),
        "records": records_out,
        "not_found": not_found,
        "not_open_access": not_open_access,
        "xml_unavailable": xml_unavailable
    }))
}

pub(super) async fn copyright(client: &PubMed, args: &Value) -> Result<Value> {
    let args: Copyright =
        serde_json::from_value(args.clone()).context("invalid copyright arguments")?;
    super::validate_pmids(&args.pmids)?;
    if args.pmids.len() > 50 {
        bail!("provide 1 to 50 PMIDs");
    }
    // EXT_ID is not unique across Europe PMC sources; the documented unique
    // MEDLINE lookup is EXT_ID:… AND SRC:MED. Without SRC:MED, a colliding
    // PMC/AGR/PPR hit can fill the bounded page and drop the PubMed row.
    let query = medline_pmid_query(&args.pmids);
    let hits = epmc_hits(&search_core(client, &query, args.pmids.len()).await?)?;
    let mut by_pmid = std::collections::HashMap::new();
    for hit in &hits {
        if !is_medline_hit(hit) {
            continue;
        }
        if let Some(id) = json_id(&hit["pmid"]).or_else(|| json_id(&hit["id"])) {
            by_pmid.insert(id, hit);
        }
    }
    let converter = ids::convert(client, &json!({"ids": args.pmids, "id_type": "pmid"})).await?;
    let mut embargo = std::collections::HashMap::new();
    if let Some(records) = converter.get("records").and_then(Value::as_array) {
        for record in records {
            if let Some(pmid) = json_id(&record["pmid"]) {
                embargo.insert(pmid, record);
            }
        }
    }
    let mut records_out = Vec::new();
    let mut missing = Vec::new();
    for pmid in &args.pmids {
        let Some(hit) = by_pmid.get(pmid) else {
            missing.push(pmid.clone());
            continue;
        };
        let pmcid = json_id(&hit["pmcid"]).and_then(|value| records::pmc_id(&value));
        let doi = json_id(&hit["doi"]);
        let license_name = json_id(&hit["license"]).filter(|name| !name.is_empty());
        let is_open_access = json_flag(&hit["isOpenAccess"]);
        let full_text_accessible = json_flag(&hit["inEPMC"]);
        let embargo_record = embargo.get(pmid);
        let reuse_permission = if license_name.is_some() {
            "license_stated"
        } else {
            "unknown"
        };
        records_out.push(json!({
            "pmid": pmid,
            "pmcid": pmcid,
            "doi": doi,
            "urls": {
                "pubmed": format!("https://pubmed.ncbi.nlm.nih.gov/{pmid}/"),
                "pmc": pmcid.as_ref().map(|id| format!("https://www.ncbi.nlm.nih.gov/pmc/articles/{id}/")),
                "doi": doi.as_ref().map(|id| format!("https://doi.org/{id}")),
                "europe_pmc": format!("https://europepmc.org/article/MED/{pmid}")
            },
            "metadata_available": true,
            "full_text_accessible": full_text_accessible,
            "is_open_access": is_open_access,
            "reuse_permission": reuse_permission,
            "license": license_name.as_ref().map(|name| json!({"name": name})),
            "embargo": embargo_record.map(|record| json!({
                "live": record.get("live"),
                "release_date": record.get("release_date")
            })),
            "notice": "An open-access flag is not a reuse license."
        }));
    }
    Ok(json!({
        "source": "Europe PMC core metadata; NCBI PMC ID Converter embargo fields",
        "sources": [
            {"name": "Europe PMC", "url": SEARCH_URL},
            {"name": "NCBI PMC ID Converter", "url": ids::SOURCE_URL}
        ],
        "contract_note": CONTRACT_NOTE,
        "requested": args.pmids,
        "returned": records_out.len(),
        "records": records_out,
        "missing_pmids": missing
    }))
}

async fn search_core(client: &PubMed, query: &str, page_size: usize) -> Result<Value> {
    let params = vec![
        ("query".into(), query.into()),
        ("resultType".into(), "core".into()),
        ("format".into(), "json".into()),
        ("pageSize".into(), page_size.max(1).to_string()),
    ];
    let value = client
        .http
        .send(
            EUROPE_PMC,
            Method::GET,
            &format!("{}search", client.europepmc),
            &params,
        )
        .await?
        .json()?;
    if value.get("error").is_some() || value.get("errMsg").is_some() || value.get("ERROR").is_some()
    {
        bail!("Europe PMC rejected the request");
    }
    Ok(value)
}

fn epmc_hits(raw: &Value) -> Result<Vec<Value>> {
    match raw.get("resultList").and_then(|v| v.get("result")) {
        None if is_zero_hits(raw) => Ok(Vec::new()),
        Some(Value::Array(items)) => Ok(items.clone()),
        Some(obj) if obj.is_object() => Ok(vec![obj.clone()]),
        Some(_) => bail!("Europe PMC returned invalid search results"),
        None => bail!("Europe PMC omitted search results"),
    }
}

fn is_zero_hits(raw: &Value) -> bool {
    match raw.get("hitCount") {
        Some(Value::Number(n)) => n.as_u64() == Some(0),
        Some(Value::String(s)) => s == "0",
        _ => false,
    }
}

fn medline_pmid_query(pmids: &[String]) -> String {
    pmids
        .iter()
        .map(|id| format!("(EXT_ID:{id} AND SRC:MED)"))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn is_medline_hit(hit: &Value) -> bool {
    hit.get("source")
        .and_then(Value::as_str)
        .is_some_and(|src| src.eq_ignore_ascii_case("MED"))
}

#[cfg(test)]
pub(super) fn medline_query_for_test(pmids: &[String]) -> String {
    medline_pmid_query(pmids)
}
