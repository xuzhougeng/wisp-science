//! Native PubMed-domain clients, independently implemented from:
//! - NCBI E-utilities (https://www.ncbi.nlm.nih.gov/books/NBK25497/,
//!   https://www.ncbi.nlm.nih.gov/books/NBK25499/)
//! - PMC ID Converter (https://pmc.ncbi.nlm.nih.gov/tools/id-converter-api/)
//! - Europe PMC Articles REST (https://europepmc.org/RestfulWebService)
//!
//! References reviewed 2026-09-06. The retired PMC OA Web Service
//! (https://pmc.ncbi.nlm.nih.gov/tools/oa-service/) is not used.
//! Tests use invented records.

mod citations;
mod europepmc;
mod ids;
mod records;
mod related;
#[cfg(test)]
mod tests;

use crate::http::{Http, NCBI};
use anyhow::{anyhow, bail, Context, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

const NCBI_EUTILS: &str = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/";
const IDCONV: &str = "https://pmc.ncbi.nlm.nih.gov/tools/idconv/api/v1/articles/";
const EUROPE_PMC_REST: &str = "https://www.ebi.ac.uk/europepmc/webservices/rest/";

pub struct PubMed {
    pub(crate) http: Http,
    api_key: Option<String>,
    email: Option<String>,
    pub(crate) ncbi: String,
    pub(crate) idconv: String,
    pub(crate) europepmc: String,
}

impl PubMed {
    /// The desktop supplies keyring-backed values; the CLI supplies its env.
    /// Credentials are held in memory and are never included in tool results.
    pub fn new(credentials: &[(String, String)]) -> Result<Self> {
        let credential = |name| {
            credentials
                .iter()
                .find(|(key, value)| key == name && !value.is_empty())
                .map(|(_, value)| value.clone())
        };
        Ok(Self {
            http: Http::new()?,
            api_key: credential("NCBI_API_KEY"),
            email: credential("NCBI_EMAIL"),
            ncbi: NCBI_EUTILS.into(),
            idconv: IDCONV.into(),
            europepmc: EUROPE_PMC_REST.into(),
        })
    }

    pub async fn call(&self, name: &str, args: &Value) -> Result<Value> {
        tokio::time::timeout(Duration::from_secs(45), self.dispatch(name, args))
            .await
            .map_err(|_| anyhow!("PubMed request exceeded 45 seconds"))?
    }

    async fn dispatch(&self, name: &str, args: &Value) -> Result<Value> {
        match name {
            "search_articles" => {
                let args: Search = serde_json::from_value(args.clone())
                    .context("invalid PubMed search arguments")?;
                let mut params = args.params()?;
                params.extend([
                    ("db".into(), "pubmed".into()),
                    ("retmode".into(), "json".into()),
                ]);
                params.extend(self.ncbi_identity());
                let raw = self.ncbi_json("esearch.fcgi", params).await?;
                search_result(&raw, &args)
            }
            "get_article_metadata" => {
                let args: Summaries = serde_json::from_value(args.clone())
                    .context("invalid PubMed metadata arguments")?;
                validate_pmids(&args.pmids)?;
                let mut params = vec![
                    ("db".into(), "pubmed".into()),
                    ("retmode".into(), "json".into()),
                    ("id".into(), args.pmids.join(",")),
                ];
                params.extend(self.ncbi_identity());
                let raw = self.ncbi_json("esummary.fcgi", params).await?;
                let mut result = summary_result(&raw, &args.pmids)?;
                result["metadata_level"] = json!("citation_and_abstract");
                let found = result["records"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|record| record["pmid"].as_str().unwrap())
                    .collect::<Vec<_>>()
                    .join(",");
                if found.is_empty() {
                    return Ok(result);
                }
                let mut fetch = vec![
                    ("db".into(), "pubmed".into()),
                    ("retmode".into(), "xml".into()),
                    ("id".into(), found),
                ];
                fetch.extend(self.ncbi_identity());
                let xml = self.ncbi_text("efetch.fcgi", fetch).await?;
                let abstracts = parse_abstracts(xml.as_bytes())?;
                for record in result["records"].as_array_mut().unwrap() {
                    let text = abstracts
                        .get(record["pmid"].as_str().unwrap())
                        .context("PubMed omitted a record when retrieving abstracts")?;
                    record["abstract"] = if text.trim().is_empty() {
                        Value::Null
                    } else {
                        json!(text)
                    };
                }
                Ok(result)
            }
            "convert_article_ids" => ids::convert(self, args).await,
            "find_related_articles" => related::find(self, args).await,
            "lookup_article_by_citation" => citations::lookup(self, args).await,
            "get_full_text_article" => europepmc::full_text(self, args).await,
            "get_copyright_status" => europepmc::copyright(self, args).await,
            _ => bail!("unknown native biological tool: {name}"),
        }
    }

    pub(super) fn ncbi_identity(&self) -> Vec<(String, String)> {
        let mut params = vec![("tool".into(), "wisp-science".into())];
        if let Some(key) = &self.api_key {
            params.push(("api_key".into(), key.clone()));
        }
        if let Some(email) = &self.email {
            params.push(("email".into(), email.clone()));
        }
        params
    }

    pub(super) async fn ncbi_json(
        &self,
        path: &str,
        params: Vec<(String, String)>,
    ) -> Result<Value> {
        let value = self
            .http
            .send(NCBI, Method::POST, &format!("{}{path}", self.ncbi), &params)
            .await?
            .json()?;
        if value.get("error").is_some() || value.get("ERROR").is_some() {
            bail!("PubMed rejected the request");
        }
        Ok(value)
    }

    pub(super) async fn ncbi_text(
        &self,
        path: &str,
        params: Vec<(String, String)>,
    ) -> Result<String> {
        self.http
            .send(NCBI, Method::POST, &format!("{}{path}", self.ncbi), &params)
            .await?
            .text()
    }
}

fn parse_abstracts(xml: &[u8]) -> Result<BTreeMap<String, String>> {
    let text = std::str::from_utf8(xml).context("invalid PubMed record XML")?;
    let doc = crate::xml::parse(text)?;
    if !doc.root_element().has_tag_name("PubmedArticleSet") {
        bail!("PubMed returned an unexpected record document");
    }
    let mut records = BTreeMap::new();
    for entry in doc
        .root_element()
        .children()
        .filter(|node| node.has_tag_name("PubmedArticle") || node.has_tag_name("PubmedBookArticle"))
    {
        let citation = crate::xml::child(entry, "MedlineCitation")
            .or_else(|| crate::xml::child(entry, "BookDocument"))
            .context("PubMed record omitted its PMID")?;
        let id =
            crate::xml::field(citation, &["PMID"]).context("PubMed record omitted its PMID")?;
        let article = crate::xml::child(citation, "Article").unwrap_or(citation);
        let parts: Vec<_> = crate::xml::child(article, "Abstract")
            .into_iter()
            .flat_map(|node| node.children())
            .filter(|node| node.has_tag_name("AbstractText"))
            .filter_map(crate::xml::text)
            .collect();
        records.insert(id, parts.join("\n"));
    }
    Ok(records)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: String,
    #[serde(default = "default_limit")]
    max_results: usize,
    #[serde(default)]
    retstart: usize,
    sort: Option<String>,
    datetype: Option<String>,
    date_from: Option<String>,
    date_to: Option<String>,
}

fn default_limit() -> usize {
    20
}

impl Search {
    fn params(&self) -> Result<Vec<(String, String)>> {
        if self.query.trim().is_empty() || self.query.len() > 8192 {
            bail!("query must contain 1 to 8192 bytes of text");
        }
        if !(1..=200).contains(&self.max_results)
            || self.retstart >= 10_000
            || self.max_results > 10_000 - self.retstart
        {
            bail!("request 1 to 200 records within PubMed's first 10,000 matches");
        }
        let sort = match self.sort.as_deref().unwrap_or("relevance") {
            "relevance" => "relevance",
            "pub_date" => "pub_date",
            "author" => "Author",
            "journal_name" => "JournalName",
            _ => bail!("sort must be relevance, pub_date, author or journal_name"),
        };
        let datetype = self.datetype.as_deref().unwrap_or("pdat");
        if !matches!(datetype, "pdat" | "edat" | "mdat") {
            bail!("datetype must be pdat, edat or mdat");
        }
        let mut params = vec![
            ("term".into(), self.query.clone()),
            ("retmax".into(), self.max_results.to_string()),
            ("retstart".into(), self.retstart.to_string()),
            ("sort".into(), sort.into()),
            ("datetype".into(), datetype.into()),
        ];
        match (&self.date_from, &self.date_to) {
            (None, None) => {}
            (Some(start), Some(end)) => {
                if !valid_date(start) || !valid_date(end) {
                    bail!("dates must be YYYY, YYYY/MM or YYYY/MM/DD");
                }
                params.push(("mindate".into(), start.clone()));
                params.push(("maxdate".into(), end.clone()));
            }
            _ => bail!("provide both date_from and date_to"),
        }
        Ok(params)
    }
}

fn valid_date(value: &str) -> bool {
    let parts: Vec<_> = value.split('/').collect();
    if !(1..=3).contains(&parts.len())
        || parts[0].len() != 4
        || parts
            .iter()
            .any(|part| !part.bytes().all(|b| b.is_ascii_digit()))
        || parts.iter().skip(1).any(|part| part.len() != 2)
    {
        return false;
    }
    let year = parts[0].parse::<i32>().unwrap_or(0);
    let month = parts.get(1).map_or(1, |part| part.parse().unwrap_or(0));
    let day = parts.get(2).map_or(1, |part| part.parse().unwrap_or(0));
    year > 0 && chrono::NaiveDate::from_ymd_opt(year, month, day).is_some()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Summaries {
    pmids: Vec<String>,
}

fn validate_pmids(pmids: &[String]) -> Result<()> {
    if !(1..=200).contains(&pmids.len()) || pmids.iter().any(|id| require_pmid(id).is_err()) {
        bail!("provide 1 to 200 positive numeric PMID strings");
    }
    Ok(())
}

fn require_pmid(id: &str) -> Result<&str> {
    if id.is_empty()
        || id.len() > 12
        || id.starts_with('0')
        || !id.bytes().all(|b| b.is_ascii_digit())
    {
        bail!("provide 1 to 200 positive numeric PMID strings");
    }
    Ok(id)
}

fn json_id(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn json_flag(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(flag) => Some(*flag),
        Value::String(text) => match text.to_ascii_lowercase().as_str() {
            "y" | "yes" | "true" | "1" => Some(true),
            "n" | "no" | "false" | "0" => Some(false),
            _ => None,
        },
        Value::Number(number) if number.as_u64() == Some(1) => Some(true),
        Value::Number(number) if number.as_u64() == Some(0) => Some(false),
        _ => None,
    }
}

fn search_result(raw: &Value, args: &Search) -> Result<Value> {
    let result = raw
        .get("esearchresult")
        .context("PubMed omitted search results")?;
    if result.get("ERROR").is_some() || result.get("errorlist").is_some() {
        bail!("PubMed rejected the search expression");
    }
    let total = result["count"]
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .context("PubMed omitted the search count")?;
    let ids: Vec<String> = serde_json::from_value(result["idlist"].clone())
        .context("PubMed returned an invalid PMID list")?;
    if !ids.is_empty() {
        validate_pmids(&ids).context("PubMed returned invalid identifiers")?;
    }
    if ids.len() > args.max_results
        || (ids.is_empty() && (args.retstart as u64) < total)
        || args.retstart as u64 + ids.len() as u64 > total.max(args.retstart as u64)
    {
        bail!("PubMed returned inconsistent pagination");
    }
    let next = args.retstart + ids.len();
    Ok(json!({
        "source": "NCBI PubMed", "query": args.query, "total": total,
        "retstart": args.retstart, "returned": ids.len(), "pmids": ids,
        "has_more": (next as u64) < total,
        "next_retstart": if !ids.is_empty() && (next as u64) < total && next < 10_000 { Some(next) } else { None },
        "retrieval_ceiling": 10_000,
        "query_translation": result.get("querytranslation")
    }))
}

fn summary_result(raw: &Value, requested: &[String]) -> Result<Value> {
    let result = raw
        .get("result")
        .and_then(Value::as_object)
        .context("PubMed omitted citation summaries")?;
    if !result.get("uids").is_some_and(Value::is_array) {
        bail!("PubMed omitted the citation identifier list");
    }
    let mut records = Vec::new();
    let mut missing = Vec::new();
    for id in requested {
        match result
            .get(id)
            .filter(|record| record.is_object() && record.get("error").is_none())
        {
            Some(record) => {
                if record["uid"].as_str() != Some(id.as_str()) {
                    bail!("PubMed returned an inconsistent citation identifier");
                }
                records.push(json!({
                    "pmid": id, "url": format!("https://pubmed.ncbi.nlm.nih.gov/{id}/"),
                    "summary": record
                }));
            }
            None => missing.push(id),
        }
    }
    Ok(json!({
        "source": "NCBI PubMed", "metadata_level": "citation_summary",
        "requested": requested, "returned": records.len(), "records": records,
        "missing_pmids": missing
    }))
}
