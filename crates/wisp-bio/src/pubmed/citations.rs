//! NCBI ECitMatch. Official reference (reviewed 2026-09-06):
//! https://www.ncbi.nlm.nih.gov/books/NBK25499/
use super::PubMed;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

pub(super) const SOURCE_URL: &str = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/ecitmatch.cgi";

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Citation {
    journal: String,
    #[serde(default)]
    year: Option<Value>,
    #[serde(default)]
    volume: Option<String>,
    #[serde(default)]
    first_page: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Lookup {
    citations: Vec<Citation>,
}

pub(super) async fn lookup(client: &PubMed, args: &Value) -> Result<Value> {
    let args: Lookup =
        serde_json::from_value(args.clone()).context("invalid citation lookup arguments")?;
    let (bdata, citations) = encode(&args.citations)?;
    let mut params = vec![
        ("db".into(), "pubmed".into()),
        ("rettype".into(), "xml".into()),
        ("bdata".into(), bdata),
    ];
    params.extend(client.ncbi_identity());
    let body = client.ncbi_text("ecitmatch.cgi", params).await?;
    citmatch_result(&body, &citations)
}

fn encode(citations: &[Citation]) -> Result<(String, Vec<Citation>)> {
    if !(1..=25).contains(&citations.len()) {
        bail!("provide 1 to 25 citations");
    }
    let mut encoded = Vec::new();
    let mut prepared = Vec::new();
    for (index, citation) in citations.iter().enumerate() {
        let journal = citation.journal.trim();
        if journal.is_empty() || journal.len() > 256 {
            bail!("each citation needs a journal name");
        }
        let mut item = citation.clone();
        if item
            .key
            .as_ref()
            .map(|k| k.trim().is_empty())
            .unwrap_or(true)
        {
            item.key = Some(format!("c{}", index + 1));
        }
        let year = year_field(&item.year)?;
        let line = format!(
            "{}|{}|{}|{}|{}|{}|",
            plus(journal),
            plus(&year),
            plus(item.volume.as_deref().unwrap_or("")),
            plus(item.first_page.as_deref().unwrap_or("")),
            plus(item.author.as_deref().unwrap_or("")),
            plus(item.key.as_deref().unwrap_or(""))
        );
        encoded.push(line);
        prepared.push(item);
    }
    Ok((encoded.join("\r"), prepared))
}

fn year_field(value: &Option<Value>) -> Result<String> {
    match value {
        None => Ok(String::new()),
        Some(Value::String(s)) => Ok(s.trim().to_string()),
        Some(Value::Number(n)) => Ok(n.to_string()),
        Some(_) => bail!("year must be an integer or string"),
    }
}

fn plus(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join("+")
}

fn citmatch_result(body: &str, requested: &[Citation]) -> Result<Value> {
    if body.trim().is_empty() {
        bail!("NCBI ECitMatch returned an empty response");
    }
    if body.trim_start().starts_with('<') {
        bail!("NCBI ECitMatch rejected the request");
    }
    let mut matched = Vec::new();
    let mut unmatched = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in body.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let fields: Vec<&str> = line.split('|').collect();
        if fields.len() < 7 {
            bail!("NCBI ECitMatch returned a malformed citation line");
        }
        let key = fields[5].replace('+', " ");
        let pmid = fields[6].trim();
        let citation = requested
            .iter()
            .find(|item| item.key.as_deref() == Some(key.trim()))
            .or_else(|| {
                requested
                    .iter()
                    .find(|item| !seen.contains(item.key.as_deref().unwrap_or("")))
            });
        let Some(citation) = citation else {
            continue;
        };
        seen.insert(citation.key.clone().unwrap_or_default());
        let record = json!({
            "key": citation.key,
            "journal": citation.journal,
            "year": year_field(&citation.year).ok(),
            "volume": citation.volume,
            "first_page": citation.first_page,
            "author": citation.author,
            "pmid": if is_pmid(pmid) { json!(pmid) } else { Value::Null },
            "url": if is_pmid(pmid) {
                json!(format!("https://pubmed.ncbi.nlm.nih.gov/{pmid}/"))
            } else {
                Value::Null
            }
        });
        if is_pmid(pmid) {
            matched.push(record);
        } else {
            let mut record = record;
            record["reason"] = json!(if pmid.is_empty() { "not matched" } else { pmid });
            unmatched.push(record);
        }
    }
    for citation in requested {
        if !seen.contains(citation.key.as_deref().unwrap_or("")) {
            unmatched.push(json!({
                "key": citation.key,
                "journal": citation.journal,
                "reason": "not matched"
            }));
        }
    }
    Ok(json!({
        "source": "NCBI ECitMatch",
        "source_url": SOURCE_URL,
        "requested": requested.len(),
        "matched": matched,
        "unmatched": unmatched
    }))
}

fn is_pmid(value: &str) -> bool {
    super::require_pmid(value).is_ok()
}

#[cfg(test)]
pub(super) fn encode_args(args: Value) -> Result<String> {
    let args: Lookup = serde_json::from_value(args)?;
    Ok(encode(&args.citations)?.0)
}

#[cfg(test)]
pub(super) fn parse_body(body: &str, args: Value) -> Result<Value> {
    let args: Lookup = serde_json::from_value(args)?;
    let (_, citations) = encode(&args.citations)?;
    citmatch_result(body, &citations)
}
