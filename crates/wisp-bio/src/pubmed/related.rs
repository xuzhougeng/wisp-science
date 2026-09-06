//! NCBI ELink related records. Official reference (reviewed 2026-09-06):
//! https://www.ncbi.nlm.nih.gov/books/NBK25499/
use super::{json_id, PubMed};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

pub(super) const SOURCE_URL: &str = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/elink.fcgi";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Related {
    pmids: Vec<String>,
    #[serde(default = "default_link")]
    link_type: String,
    #[serde(default = "default_limit")]
    max_results: usize,
}

fn default_link() -> String {
    "pubmed_pubmed".into()
}

fn default_limit() -> usize {
    20
}

struct Spec {
    link_name: &'static str,
    db: &'static str,
    cmd: &'static str,
    max_results: usize,
    pmids: Vec<String>,
}

pub(super) async fn find(client: &PubMed, args: &Value) -> Result<Value> {
    let args: Related =
        serde_json::from_value(args.clone()).context("invalid related-article arguments")?;
    let spec = spec(&args)?;
    let mut params = vec![
        ("dbfrom".into(), "pubmed".into()),
        ("db".into(), spec.db.into()),
        ("linkname".into(), spec.link_name.into()),
        ("cmd".into(), spec.cmd.into()),
        ("id".into(), spec.pmids.join(",")),
        ("retmode".into(), "json".into()),
    ];
    params.extend(client.ncbi_identity());
    let raw = client.ncbi_json("elink.fcgi", params).await?;
    related_result(&raw, &spec)
}

fn spec(args: &Related) -> Result<Spec> {
    super::validate_pmids(&args.pmids)?;
    if args.pmids.len() > 20 {
        bail!("provide 1 to 20 PMIDs");
    }
    if !(1..=200).contains(&args.max_results) {
        bail!("max_results must be 1 to 200");
    }
    let (link_name, db, cmd) = match args.link_type.as_str() {
        "pubmed_pubmed" => ("pubmed_pubmed", "pubmed", "neighbor_score"),
        "pubmed_pmc" => ("pubmed_pmc", "pmc", "neighbor"),
        "pubmed_gene" => ("pubmed_gene", "gene", "neighbor"),
        "pubmed_protein" => ("pubmed_protein", "protein", "neighbor"),
        "pubmed_nucleotide" => ("pubmed_nucleotide", "nuccore", "neighbor"),
        _ => bail!(
            "link_type must be pubmed_pubmed, pubmed_pmc, pubmed_gene, pubmed_protein or pubmed_nucleotide"
        ),
    };
    Ok(Spec {
        link_name,
        db,
        cmd,
        max_results: args.max_results,
        pmids: args.pmids.clone(),
    })
}

fn related_result(raw: &Value, spec: &Spec) -> Result<Value> {
    let linksets = raw
        .get("linksets")
        .and_then(Value::as_array)
        .context("NCBI ELink omitted linksets")?;
    if linksets
        .iter()
        .any(|set| set.get("ERROR").is_some() || set.get("error").is_some())
    {
        bail!("NCBI ELink rejected the request");
    }
    let mut links = Vec::new();
    for set in linksets {
        let dbs = set
            .get("linksetdbs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let matched = dbs
            .iter()
            .find(|db| db.get("linkname").and_then(Value::as_str) == Some(spec.link_name));
        let Some(db) = matched.or_else(|| dbs.first()) else {
            continue;
        };
        let Some(entries) = db.get("links").and_then(Value::as_array) else {
            continue;
        };
        for entry in entries {
            let (id, score) = match entry {
                Value::String(id) => (Some(id.clone()), None),
                Value::Number(n) => (Some(n.to_string()), None),
                Value::Object(obj) => (
                    obj.get("id").and_then(json_id),
                    obj.get("score").and_then(json_id),
                ),
                _ => (None, None),
            };
            let Some(id) = id.filter(|id| !id.is_empty()) else {
                continue;
            };
            links.push((id, score));
        }
    }
    let has_more = links.len() > spec.max_results;
    links.truncate(spec.max_results);
    let records: Vec<Value> = links
        .into_iter()
        .map(|(id, score)| {
            json!({
                "id": id,
                "score": score,
                "url": record_url(spec.db, &id)
            })
        })
        .collect();
    Ok(json!({
        "source": "NCBI ELink",
        "source_url": SOURCE_URL,
        "link_name": spec.link_name,
        "dbfrom": "pubmed",
        "dbto": spec.db,
        "requested_pmids": spec.pmids,
        "returned": records.len(),
        "has_more": has_more,
        "ranking": "upstream_elink_order",
        "records": records
    }))
}

fn record_url(db: &str, id: &str) -> String {
    match db {
        "pubmed" => format!("https://pubmed.ncbi.nlm.nih.gov/{id}/"),
        "pmc" => {
            let pmcid = super::records::pmc_id(id).unwrap_or_else(|| format!("PMC{id}"));
            format!("https://www.ncbi.nlm.nih.gov/pmc/articles/{pmcid}/")
        }
        "gene" => format!("https://www.ncbi.nlm.nih.gov/gene/{id}"),
        "protein" => format!("https://www.ncbi.nlm.nih.gov/protein/{id}"),
        _ => format!("https://www.ncbi.nlm.nih.gov/nuccore/{id}"),
    }
}

#[cfg(test)]
pub(super) fn parse_spec(args: Value) -> Result<(String, usize, Vec<String>)> {
    let args: Related = serde_json::from_value(args)?;
    let spec = spec(&args)?;
    Ok((spec.link_name.into(), spec.max_results, spec.pmids))
}

#[cfg(test)]
pub(super) fn parse_related(raw: &Value, args: Value) -> Result<Value> {
    let args: Related = serde_json::from_value(args)?;
    related_result(raw, &spec(&args)?)
}
