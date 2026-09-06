//! Native `clinical-genomics` domain against CIViC, ClinGen and Open Targets.
//! Independently implemented from:
//!
//! - [CIViC API](https://docs.civicdb.org/en/latest/api.html)
//! - [CIViC GraphQL (v2)](https://griffithlab.github.io/civic-v2/)
//! - [ClinGen downloads and APIs](https://search.clinicalgenome.org/kb/downloads)
//! - [ClinGen actionability JSON](https://actionability.clinicalgenome.org/ac/Adult/api/summ?flavor=flat)
//! - [ClinGen Evidence Repository](https://erepo.clinicalgenome.org/evrepo/api/summary/srvc)
//! - [Open Targets Platform GraphQL](https://platform-docs.opentargets.org/data-access/graphql-api)
//!
//! References reviewed 2026-09-06. Tests use invented records.

mod civic;
mod clingen;
mod open_targets;
#[cfg(test)]
mod tests;

use crate::http::{Source, MAX_RESPONSE};
use crate::NativeBio;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::header::RETRY_AFTER;
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use std::time::Duration;
use wisp_llm::ToolSchema;

const CIVIC_GRAPHQL: &str = "https://civicdb.org/api/graphql";
const CIVIC_WEB: &str = "https://civicdb.org";
const CLINGEN_SEARCH: &str = "https://search.clinicalgenome.org";
const CLINGEN_ACTIONABILITY: &str = "https://actionability.clinicalgenome.org";
const CLINGEN_EREPO: &str = "https://erepo.genome.network/evrepo/api";
const OPEN_TARGETS_GRAPHQL: &str = "https://api.platform.opentargets.org/api/v4/graphql";

const CIVIC: Source = Source("CIViC", Duration::from_millis(350));
const CLINGEN: Source = Source("ClinGen", Duration::from_millis(500));
const OPEN_TARGETS: Source = Source("Open Targets", Duration::from_millis(500));

const DEFAULT_PAGE: u32 = 25;
const CIVIC_MAX_PAGE: u32 = 100;
const CLINGEN_MAX_PAGE: u32 = 200;
const OT_MAX_PAGE: u32 = 100;
const MAX_TEXT: usize = 256;
const MAX_QUERY: usize = 8192;
const MAX_VARIABLES: usize = 16 * 1024;

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    let mut out = civic::catalog();
    out.extend(clingen::catalog());
    out.extend(open_targets::catalog());
    out
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        n if n.starts_with("civic_") => civic::call(bio, name, args).await,
        n if n.starts_with("clingen_") => clingen::call(bio, name, args).await,
        n if n.starts_with("open_targets_") => open_targets::call(bio, name, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

fn tool(
    name: &'static str,
    description: &'static str,
    parameters: Value,
) -> (&'static str, ToolSchema) {
    (
        "clinical-genomics",
        ToolSchema::new(name, description, parameters),
    )
}

fn civic_endpoint(bio: &NativeBio) -> String {
    override_url(bio, "CIVIC_GRAPHQL_URL", CIVIC_GRAPHQL)
}

fn clingen_search(bio: &NativeBio) -> String {
    override_url(bio, "CLINGEN_SEARCH_URL", CLINGEN_SEARCH)
}

fn clingen_actionability(bio: &NativeBio) -> String {
    override_url(bio, "CLINGEN_ACTIONABILITY_URL", CLINGEN_ACTIONABILITY)
}

fn clingen_erepo(bio: &NativeBio) -> String {
    override_url(bio, "CLINGEN_EREPO_URL", CLINGEN_EREPO)
}

fn open_targets_endpoint(bio: &NativeBio) -> String {
    override_url(bio, "OPEN_TARGETS_GRAPHQL_URL", OPEN_TARGETS_GRAPHQL)
}

fn override_url(bio: &NativeBio, key: &str, fallback: &str) -> String {
    bio.credential(key)
        .map(|value| value.trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn default_page() -> u32 {
    DEFAULT_PAGE
}

fn bound_page(n: u32, max: u32) -> Result<u32> {
    if !(1..=max).contains(&n) {
        bail!("max_results must be between 1 and {max}");
    }
    Ok(n)
}

fn require_text(value: &str, what: &str, max: usize) -> Result<String> {
    let text = value.trim();
    if text.is_empty() || text.len() > max {
        bail!("{what} must contain 1 to {max} characters");
    }
    if text.chars().any(|c| c.is_control()) {
        bail!("{what} contains control characters");
    }
    Ok(text.to_string())
}

fn require_symbol(value: &str, what: &str) -> Result<String> {
    let text = require_text(value, what, 64)?;
    let ok = text.chars().enumerate().all(|(i, c)| {
        c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || (i > 0 && c == ':')
    });
    if !ok {
        bail!("{what} {text:?} is not a gene symbol or ClinGen region id");
    }
    Ok(text)
}

fn require_id(value: i64, what: &str) -> Result<i32> {
    if value < 1 || value > i32::MAX as i64 {
        bail!("{what} must be a positive integer identifier");
    }
    Ok(value as i32)
}

fn page(
    source: &str,
    source_url: &str,
    query: Value,
    mut records: Vec<Value>,
    total: u64,
    cap: u32,
    has_more: bool,
) -> Value {
    let cap = cap as usize;
    let truncated = records.len() > cap || has_more || (records.len() as u64) < total;
    if records.len() > cap {
        records.truncate(cap);
    }
    json!({
        "source": source,
        "source_url": source_url,
        "query": query,
        "total_count": total,
        "returned": records.len(),
        "truncated": truncated,
        "has_more": has_more || (records.len() as u64) < total,
        "records": records
    })
}

async fn get_json(
    bio: &NativeBio,
    source: Source,
    url: &str,
    params: &[(String, String)],
) -> Result<Value> {
    let response = bio.http().send(source, Method::GET, url, params).await?;
    response.check()?;
    serde_json::from_slice(&response.body)
        .with_context(|| format!("{} returned invalid JSON", source.0))
}

async fn graphql(
    bio: &NativeBio,
    source: Source,
    url: &str,
    query: &str,
    variables: Value,
    bearer: Option<&str>,
    fail_on_errors: bool,
) -> Result<Value> {
    let payload = json!({"query": query, "variables": variables});
    let mut last_transient = None;
    for attempt in 0..2 {
        let body = json_post(bio, source, url, &payload, bearer).await?;
        if transient_graphql(&body) && attempt == 0 {
            last_transient = Some(body);
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        if fail_on_errors {
            if let Some(errors) = body.get("errors").and_then(Value::as_array) {
                if !errors.is_empty() {
                    bail!("{} GraphQL query was rejected", source.0);
                }
            }
        }
        return Ok(body);
    }
    last_transient.context("GraphQL retry exhausted")
}

async fn json_post(
    bio: &NativeBio,
    source: Source,
    url: &str,
    body: &Value,
    bearer: Option<&str>,
) -> Result<Value> {
    let bytes = serde_json::to_vec(body).context("failed to encode GraphQL request")?;
    for attempt in 0..2 {
        let mut request = bio
            .http()
            .0
            .post(url)
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .body(bytes.clone());
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| anyhow!("{} connection failed or timed out", source.0))?;
        let status = response.status();
        if attempt == 0 && (status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()) {
            let delay = response
                .headers()
                .get(RETRY_AFTER)
                .map(|header| header.to_str().ok().and_then(retry_delay))
                .unwrap_or(Some(2));
            if let Some(delay) = delay.filter(|seconds| *seconds <= 5) {
                drop(response);
                tokio::time::sleep(Duration::from_secs(delay)).await;
                continue;
            }
        }
        if !status.is_success() {
            bail!("{} returned HTTP {}", source.0, status.as_u16());
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE as u64)
        {
            bail!(
                "{} response exceeded 4 MiB; request fewer records",
                source.0
            );
        }
        let mut buf = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow!("{} response could not be read", source.0))?
        {
            if buf.len() + chunk.len() > MAX_RESPONSE {
                bail!(
                    "{} response exceeded 4 MiB; request fewer records",
                    source.0
                );
            }
            buf.extend_from_slice(&chunk);
        }
        return serde_json::from_slice(&buf)
            .with_context(|| format!("{} returned invalid JSON", source.0));
    }
    unreachable!("second attempt returns a response")
}

fn retry_delay(value: &str) -> Option<u64> {
    value.parse().ok().or_else(|| {
        chrono::DateTime::parse_from_rfc2822(value)
            .ok()
            .map(|date| (date.timestamp() - chrono::Utc::now().timestamp()).max(0) as u64)
    })
}

fn transient_graphql(body: &Value) -> bool {
    let Some(errors) = body.get("errors").and_then(Value::as_array) else {
        return false;
    };
    !errors.is_empty()
        && errors.iter().all(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .is_some_and(|message| {
                    message
                        .to_ascii_lowercase()
                        .contains("internal server error")
                })
        })
}

fn graphql_data<'a>(payload: &'a Value, source: &str) -> Result<&'a Value> {
    payload
        .get("data")
        .with_context(|| format!("{source} omitted GraphQL data"))
}

fn connection(payload: &Value, field: &str, source: &str) -> Result<(Vec<Value>, u64, bool)> {
    let data = graphql_data(payload, source)?;
    let conn = data
        .get(field)
        .with_context(|| format!("{source} omitted {field}"))?;
    if conn.is_null() {
        bail!("{source} omitted {field}");
    }
    let nodes = conn
        .get("nodes")
        .and_then(Value::as_array)
        .with_context(|| format!("{source} omitted {field} nodes"))?;
    let total = json_u64(conn.get("totalCount"))
        .with_context(|| format!("{source} omitted {field} totalCount"))?;
    let has_more = conn
        .get("pageInfo")
        .and_then(|info| info.get("hasNextPage"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let records = nodes
        .iter()
        .filter(|node| node.is_object())
        .cloned()
        .collect();
    Ok((records, total, has_more))
}

fn node(payload: &Value, field: &str, source: &str) -> Result<Option<Value>> {
    let data = graphql_data(payload, source)?;
    match data.get(field) {
        None => bail!("{source} omitted {field}"),
        Some(Value::Null) => Ok(None),
        Some(value) if value.is_object() => Ok(Some(value.clone())),
        Some(_) => bail!("{source} returned an invalid {field} record"),
    }
}

fn json_u64(value: Option<&Value>) -> Option<u64> {
    match value {
        Some(Value::Number(number)) => number
            .as_u64()
            .or_else(|| number.as_i64().and_then(|n| u64::try_from(n).ok())),
        Some(Value::String(text)) => text.parse().ok(),
        _ => None,
    }
}

fn civic_url(link: Option<&Value>) -> Value {
    match link.and_then(Value::as_str) {
        Some(path) if path.starts_with("http://") || path.starts_with("https://") => json!(path),
        Some(path) if path.starts_with('/') => json!(format!("{CIVIC_WEB}{path}")),
        Some(path) if !path.is_empty() => json!(format!("{CIVIC_WEB}/{path}")),
        _ => Value::Null,
    }
}

fn with_civic_url(mut record: Value) -> Value {
    record["url"] = civic_url(record.get("link"));
    record
}
