use super::{
    bound_u32, grants_origin, json_text, json_u64, pipe_join, require_text, DEFAULT_GRANT_RECORDS,
    GRANTS, GRANTS_SITE, MAX_GRANT_RECORDS, MAX_QUERY,
};
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Map, Value};

const STATUSES: &[&str] = &["forecasted", "posted", "closed", "archived"];
const DEFAULT_STATUSES: &[&str] = &["forecasted", "posted"];
const FACET_KEYS: &[&str] = &[
    "oppStatusOptions",
    "agencies",
    "eligibilities",
    "fundingCategories",
    "fundingInstruments",
    "dateRangeOptions",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Search {
    keyword: Option<String>,
    opportunity_number: Option<String>,
    aln: Option<String>,
    agencies: Option<Vec<String>>,
    opportunity_statuses: Option<Vec<String>>,
    eligibilities: Option<Vec<String>>,
    funding_categories: Option<Vec<String>>,
    funding_instruments: Option<Vec<String>>,
    #[serde(default)]
    count_only: bool,
    #[serde(default = "default_max_records")]
    max_records: u32,
    #[serde(default)]
    start_record: u32,
    #[serde(default = "default_true")]
    include_facets: bool,
}

fn default_max_records() -> u32 {
    DEFAULT_GRANT_RECORDS
}

fn default_true() -> bool {
    true
}

pub async fn search(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Search =
        serde_json::from_value(args.clone()).context("invalid Grants.gov search arguments")?;
    let spec = Spec::from_args(&args)?;
    if args.count_only {
        let data = post(bio, spec.payload(0, 0)).await?;
        let hit_count = hit_count(&data)?;
        let mut result = json!({
            "source": "Grants.gov",
            "source_url": GRANTS_SITE,
            "query": spec.query_json(),
            "total": hit_count,
            "returned": 0,
            "truncated": false,
            "has_more": hit_count > 0,
            "next_start_record": if hit_count > 0 { Value::from(0) } else { Value::Null },
            "records": []
        });
        if args.include_facets {
            result["facets"] = facets(&data);
        }
        return Ok(result);
    }
    let cap = bound_u32(args.max_records, 1, MAX_GRANT_RECORDS, "max_records")? as usize;
    let start = bound_u32(args.start_record, 0, 100_000, "start_record")? as usize;
    let mut records = Vec::new();
    let mut total = None;
    let mut first_facets = Value::Null;
    let mut cursor = start;
    while records.len() < cap {
        let rows = (cap - records.len()).min(100);
        let data = post(bio, spec.payload(rows, cursor)).await?;
        if total.is_none() {
            total = Some(hit_count(&data)?);
            first_facets = facets(&data);
        }
        let hits = opp_hits(&data)?;
        if hits.is_empty() {
            if total == Some(0) || !records.is_empty() {
                break;
            }
            bail!("Grants.gov reported hits but returned no opportunity rows");
        }
        for hit in hits {
            records.push(project_opportunity(&hit));
            cursor += 1;
            if records.len() >= cap {
                break;
            }
        }
        if (cursor as u64) >= total.unwrap_or(0) {
            break;
        }
    }
    let total = total.context("Grants.gov omitted hitCount")?;
    let returned = records.len() as u64;
    let has_more = start as u64 + returned < total;
    let mut result = json!({
        "source": "Grants.gov",
        "source_url": GRANTS_SITE,
        "query": spec.query_json(),
        "total": total,
        "returned": returned,
        "start_record": start,
        "truncated": has_more,
        "has_more": has_more,
        "next_start_record": if has_more { Value::from(start as u64 + returned) } else { Value::Null },
        "records": records
    });
    if args.include_facets {
        result["facets"] = first_facets;
    }
    Ok(result)
}

struct Spec {
    keyword: Option<String>,
    opportunity_number: Option<String>,
    aln: Option<String>,
    agencies: Option<String>,
    statuses: String,
    eligibilities: Option<String>,
    funding_categories: Option<String>,
    funding_instruments: Option<String>,
}

impl Spec {
    fn from_args(args: &Search) -> Result<Self> {
        let keyword = optional_text(args.keyword.as_deref(), "keyword", MAX_QUERY)?;
        let opportunity_number =
            optional_text(args.opportunity_number.as_deref(), "opportunity_number", 64)?;
        let aln = optional_text(args.aln.as_deref(), "aln", 16)?;
        let agencies = optional_join(args.agencies.as_deref(), "agencies")?;
        let eligibilities = optional_join(args.eligibilities.as_deref(), "eligibilities")?;
        let funding_categories =
            optional_join(args.funding_categories.as_deref(), "funding_categories")?;
        let funding_instruments =
            optional_join(args.funding_instruments.as_deref(), "funding_instruments")?;
        if keyword.is_none()
            && opportunity_number.is_none()
            && aln.is_none()
            && agencies.is_none()
            && eligibilities.is_none()
            && funding_categories.is_none()
            && funding_instruments.is_none()
        {
            bail!("search_grants requires at least one of keyword, opportunity_number, aln, agencies, eligibilities, funding_categories or funding_instruments");
        }
        Ok(Self {
            keyword,
            opportunity_number,
            aln,
            agencies,
            statuses: statuses(args.opportunity_statuses.as_deref())?,
            eligibilities,
            funding_categories,
            funding_instruments,
        })
    }

    fn payload(&self, rows: usize, start_record_num: usize) -> Value {
        let mut body = json!({
            "rows": rows,
            "startRecordNum": start_record_num,
            "oppStatuses": self.statuses,
        });
        if let Some(keyword) = &self.keyword {
            body["keyword"] = json!(keyword);
        }
        if let Some(number) = &self.opportunity_number {
            body["oppNum"] = json!(number);
        }
        if let Some(aln) = &self.aln {
            body["aln"] = json!(aln);
        }
        if let Some(agencies) = &self.agencies {
            body["agencies"] = json!(agencies);
        }
        if let Some(eligibilities) = &self.eligibilities {
            body["eligibilities"] = json!(eligibilities);
        }
        if let Some(categories) = &self.funding_categories {
            body["fundingCategories"] = json!(categories);
        }
        if let Some(instruments) = &self.funding_instruments {
            body["fundingInstruments"] = json!(instruments);
        }
        body
    }

    fn query_json(&self) -> Value {
        json!({
            "keyword": self.keyword,
            "opportunity_number": self.opportunity_number,
            "aln": self.aln,
            "agencies": self.agencies,
            "opportunity_statuses": self.statuses,
            "eligibilities": self.eligibilities,
            "funding_categories": self.funding_categories,
            "funding_instruments": self.funding_instruments
        })
    }
}

async fn post(bio: &NativeBio, payload: Value) -> Result<Value> {
    let url = format!("{}/v1/api/search2", grants_origin(bio));
    let envelope = bio.http().send_json(GRANTS, &url, &payload).await?.json()?;
    if !errorcode_ok(&envelope) {
        bail!("Grants.gov rejected the request");
    }
    match envelope.get("data") {
        Some(Value::Object(_)) => Ok(envelope["data"].clone()),
        _ => bail!("Grants.gov returned an unexpected envelope"),
    }
}

fn errorcode_ok(envelope: &Value) -> bool {
    match envelope.get("errorcode") {
        Some(Value::Number(number)) => number.as_i64() == Some(0),
        Some(Value::String(text)) => text == "0",
        _ => false,
    }
}

fn hit_count(data: &Value) -> Result<u64> {
    data.get("hitCount")
        .and_then(json_u64)
        .context("Grants.gov omitted hitCount")
}

fn opp_hits(data: &Value) -> Result<Vec<Value>> {
    match data.get("oppHits") {
        Some(Value::Array(hits)) => Ok(hits.clone()),
        Some(_) => bail!("Grants.gov returned an unexpected oppHits value"),
        None => Ok(Vec::new()),
    }
}

fn facets(data: &Value) -> Value {
    let mut out = Map::new();
    for key in FACET_KEYS {
        if let Some(value) = data.get(*key) {
            out.insert((*key).to_string(), value.clone());
        }
    }
    Value::Object(out)
}

fn project_opportunity(raw: &Value) -> Value {
    let id = json_text(&raw["id"]);
    let url = id
        .as_deref()
        .map(|id| format!("{GRANTS_SITE}/search-results-detail/{id}"));
    json!({
        "id": id,
        "number": json_text(&raw["number"]),
        "title": json_text(&raw["title"]),
        "agency_code": json_text(&raw["agencyCode"]),
        "agency_name": json_text(&raw["agencyName"]),
        "status": json_text(&raw["oppStatus"]),
        "open_date": json_text(&raw["openDate"]),
        "close_date": json_text(&raw["closeDate"]),
        "document_type": json_text(&raw["docType"]),
        "assistance_listings": raw.get("alnist").cloned().unwrap_or(Value::Null),
        "url": url
    })
}

fn statuses(values: Option<&[String]>) -> Result<String> {
    let list = match values {
        Some(values) => values.iter().map(String::as_str).collect::<Vec<_>>(),
        None => DEFAULT_STATUSES.to_vec(),
    };
    if list.is_empty() {
        bail!("opportunity_statuses must not be empty");
    }
    let mut parts = Vec::new();
    for status in list {
        if !STATUSES.contains(&status) {
            bail!("opportunity_statuses values must be forecasted, posted, closed or archived");
        }
        if !parts.iter().any(|existing| existing == status) {
            parts.push(status.to_string());
        }
    }
    Ok(parts.join("|"))
}

fn optional_text(value: Option<&str>, field: &str, max: usize) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(text) if text.trim().is_empty() => {
            bail!("{field} must contain 1 to {max} characters")
        }
        Some(text) => Ok(Some(require_text(text, field, max)?)),
    }
}

fn optional_join(values: Option<&[String]>, field: &str) -> Result<Option<String>> {
    match values {
        None => Ok(None),
        Some(values) => Ok(Some(pipe_join(values, field)?)),
    }
}
