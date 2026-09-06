use super::{bound_u32, cred_base, looks_like_html};
use crate::http::Source;
use crate::NativeBio;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration;

const KEGG: Source = Source("KEGG", Duration::from_millis(350));
const KEGG_HOST: &str = "https://rest.kegg.jp";
const KEGG_PAGE: &str = "https://www.kegg.jp";
const MAX_IDS: usize = 50;
const BATCH: usize = 10;
const MAX_ID_LEN: usize = 64;
const MAX_DB_LEN: usize = 32;
const MAX_QUERY: usize = 256;
const MAX_HITS: u32 = 200;
const DEFAULT_HITS: u32 = 50;
const FIELD_WIDTH: usize = 12;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetKegg {
    ids: Vec<String>,
    #[serde(default)]
    include_raw: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchKegg {
    query: String,
    #[serde(default = "default_database")]
    database: String,
    option: Option<String>,
    #[serde(default)]
    exact_gene_symbol: bool,
    #[serde(default = "default_max_hits")]
    max_hits: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkKegg {
    ids: Vec<String>,
    target_db: String,
    #[serde(default = "default_operation")]
    operation: String,
}

fn default_database() -> String {
    "hsa".into()
}
fn default_max_hits() -> u32 {
    DEFAULT_HITS
}
fn default_operation() -> String {
    "link".into()
}

pub(super) async fn get_kegg_entries(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetKegg =
        serde_json::from_value(args.clone()).context("invalid get_kegg_entries arguments")?;
    let ids = require_kegg_ids(&args.ids)?;
    let base = kegg_base(bio);
    let mut fetched = Vec::new();
    let mut source_url = String::new();
    for chunk in ids.chunks(BATCH) {
        let joined = join_dbentries(chunk);
        if source_url.is_empty() {
            source_url = format!("{KEGG_HOST}/get/{joined}");
        }
        let url = format!("{base}/get/{joined}");
        let text = kegg_text(bio, &url, true).await?;
        if text.trim().is_empty() {
            continue;
        }
        fetched.extend(parse_get_body(&text)?);
    }
    let mut unused = fetched;
    let mut records = Vec::new();
    let mut missing = Vec::new();
    for id in &ids {
        match unused.iter().position(|(record, _)| record.matches(id)) {
            Some(index) => {
                let (record, raw) = unused.remove(index);
                records.push(record.into_json(id, args.include_raw.then_some(raw)));
            }
            None => missing.push(id.clone()),
        }
    }
    if !missing.is_empty() {
        bail!("KEGG returned no entry for {}", missing.join(", "));
    }
    Ok(json!({
        "source": "KEGG",
        "source_url": source_url,
        "ids": ids,
        "returned": records.len(),
        "records": records
    }))
}

pub(super) async fn search_kegg(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchKegg =
        serde_json::from_value(args.clone()).context("invalid search_kegg arguments")?;
    let query = require_query(&args.query)?;
    let database = require_kegg_token(&args.database, MAX_DB_LEN, "database")?;
    let option = args
        .option
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(require_find_option)
        .transpose()?;
    if option.is_some() && !formula_database(&database) {
        bail!("option formula, exact_mass and mol_weight are valid only for compound or drug");
    }
    let cap = bound_u32(args.max_hits, 1, MAX_HITS, "max_hits")?;
    let mut path = format!(
        "find/{}/{}",
        encode_kegg_segment(&database),
        encode_kegg_segment(&query)
    );
    if let Some(option) = option.as_deref() {
        path.push('/');
        path.push_str(&encode_kegg_segment(option));
    }
    let text = kegg_text(bio, &format!("{}/{path}", kegg_base(bio)), true).await?;
    let mut hits = parse_find(&text)?;
    if args.exact_gene_symbol {
        hits.retain(|(_, description)| exact_symbol_match(description, &query));
    }
    let total = hits.len();
    let truncated = total > cap as usize;
    let page: Vec<Value> = hits
        .into_iter()
        .take(cap as usize)
        .map(|(entry_id, description)| json!({"entry_id": entry_id, "description": description}))
        .collect();
    let mut out = json!({
        "source": "KEGG",
        "source_url": format!("{KEGG_HOST}/{path}"),
        "query": query,
        "database": database,
        "option": option,
        "exact_gene_symbol": args.exact_gene_symbol,
        "total_hits": total,
        "returned": page.len(),
        "truncated": truncated,
        "records": page
    });
    if args.exact_gene_symbol {
        out["n_matches"] = json!(total);
    }
    Ok(out)
}

pub(super) async fn link_kegg_ids(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: LinkKegg =
        serde_json::from_value(args.clone()).context("invalid link_kegg_ids arguments")?;
    let ids = require_kegg_ids(&args.ids)?;
    let target_db = require_kegg_token(&args.target_db, MAX_DB_LEN, "target_db")?;
    let operation = require_operation(&args.operation)?;
    let base = kegg_base(bio);
    let mut pairs = Vec::new();
    let mut source_url = String::new();
    for chunk in ids.chunks(BATCH) {
        let joined = join_dbentries(chunk);
        let path = format!("{operation}/{}/{}", encode_kegg_segment(&target_db), joined);
        if source_url.is_empty() {
            source_url = format!("{KEGG_HOST}/{path}");
        }
        let text = kegg_text(bio, &format!("{base}/{path}"), true).await?;
        if text.trim().is_empty() {
            continue;
        }
        pairs.extend(parse_two_column(&text)?);
    }
    let mut hit: HashSet<String> = HashSet::new();
    let mut records = Vec::new();
    for (left, right) in pairs {
        let source_id = map_query_id(&left, &ids);
        hit.insert(source_id.clone());
        records.push(json!({"source_id": source_id, "target_id": right}));
    }
    let missing: Vec<String> = ids
        .iter()
        .filter(|id| !hit.contains(*id))
        .cloned()
        .collect();
    Ok(json!({
        "source": "KEGG",
        "source_url": source_url,
        "operation": operation,
        "target_db": target_db,
        "query_ids": ids,
        "returned": records.len(),
        "missing_ids": missing,
        "records": records
    }))
}

async fn kegg_text(bio: &NativeBio, url: &str, empty_on_404: bool) -> Result<String> {
    let response = bio.http().send(KEGG, Method::GET, url, &[]).await?;
    match response.status {
        StatusCode::OK => {
            if looks_like_html(&response.body) {
                bail!("KEGG returned HTML instead of text");
            }
            String::from_utf8(response.body).context("KEGG returned invalid UTF-8")
        }
        StatusCode::NOT_FOUND if empty_on_404 => Ok(String::new()),
        StatusCode::BAD_REQUEST => bail!("KEGG rejected the request (HTTP 400)"),
        status => bail!("KEGG returned HTTP {}", status.as_u16()),
    }
}

fn kegg_base(bio: &NativeBio) -> String {
    cred_base(bio, "KEGG_BASE_URL", KEGG_HOST)
}

fn require_kegg_ids(values: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for value in values {
        let id = require_kegg_token(value, MAX_ID_LEN, "KEGG id")?;
        if !seen.insert(id.clone()) {
            bail!("duplicate KEGG id {id}");
        }
        out.push(id);
    }
    if out.is_empty() {
        bail!("provide at least one KEGG id");
    }
    if out.len() > MAX_IDS {
        bail!("{} ids exceeds the per-call bound of {MAX_IDS}", out.len());
    }
    Ok(out)
}

fn require_kegg_token(value: &str, max: usize, what: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > max {
        bail!("{what} must contain 1 to {max} characters");
    }
    if trimmed.contains("..")
        || trimmed.contains('/')
        || trimmed.contains('+')
        || trimmed
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_' | '.')))
    {
        bail!("{what} {trimmed:?} is not a KEGG identifier");
    }
    Ok(trimmed.to_string())
}

fn require_query(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_QUERY {
        bail!("query must contain 1 to {MAX_QUERY} characters");
    }
    if trimmed.contains("..") || trimmed.contains('/') {
        bail!("query must not contain '/' or '..'");
    }
    Ok(trimmed.to_string())
}

fn require_operation(value: &str) -> Result<&'static str> {
    match value.trim() {
        "link" => Ok("link"),
        "conv" => Ok("conv"),
        other => bail!("operation must be link or conv (got {other})"),
    }
}

fn require_find_option(value: &str) -> Result<String> {
    match value {
        "formula" | "exact_mass" | "mol_weight" => Ok(value.to_string()),
        other => bail!("option must be formula, exact_mass or mol_weight (got {other})"),
    }
}

fn formula_database(database: &str) -> bool {
    matches!(database, "compound" | "cpd" | "drug" | "dr")
}

fn join_dbentries(ids: &[String]) -> String {
    ids.iter()
        .map(|id| encode_kegg_segment(id))
        .collect::<Vec<_>>()
        .join("+")
}

fn encode_kegg_segment(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' | b'+' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn parse_get_body(text: &str) -> Result<Vec<(KeggRecord, String)>> {
    if looks_like_html(text.as_bytes()) {
        bail!("KEGG returned HTML instead of text");
    }
    let chunks = split_flat_records(text)?;
    if chunks.is_empty() && !text.trim().is_empty() {
        bail!("KEGG returned an unusable response");
    }
    let mut out = Vec::new();
    for chunk in chunks {
        out.push((parse_flat_record(&chunk)?, chunk));
    }
    Ok(out)
}

fn split_flat_records(text: &str) -> Result<Vec<String>> {
    let mut records = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        if current.is_empty() && line.trim().is_empty() {
            continue;
        }
        current.push_str(line);
        current.push('\n');
        if line.trim() == "///" {
            records.push(std::mem::take(&mut current));
        }
    }
    if current.trim().is_empty() {
        Ok(records)
    } else {
        Err(anyhow!("KEGG returned an unusable response"))
    }
}

fn parse_flat_record(chunk: &str) -> Result<KeggRecord> {
    let mut fields: Vec<(String, Vec<String>)> = Vec::new();
    let mut current: Option<usize> = None;
    for line in chunk.lines() {
        if line.trim() == "///" {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let keyword = field_keyword(line);
        let value = field_value(line);
        if keyword.is_empty() {
            if let Some(index) = current {
                fields[index].1.push(value);
            }
        } else {
            fields.push((keyword, vec![value]));
            current = Some(fields.len() - 1);
        }
    }
    let entry = field_lines(&fields, "ENTRY")
        .first()
        .map(|line| line.as_str())
        .unwrap_or("");
    let mut tokens = entry.split_whitespace();
    let entry_id = tokens.next().unwrap_or("").to_string();
    let entry_type = tokens.next().unwrap_or("").to_string();
    if entry_id.is_empty() {
        bail!("KEGG returned an unusable response");
    }
    Ok(KeggRecord {
        entry_id,
        entry_type,
        name: parse_names(field_lines(&fields, "NAME")),
        symbol: parse_symbols(field_lines(&fields, "SYMBOL")),
        definition: joined_field(&fields, "DEFINITION")
            .or_else(|| joined_field(&fields, "DESCRIPTION")),
        organism: joined_field(&fields, "ORGANISM"),
        formula: joined_field(&fields, "FORMULA"),
        pathway: parse_id_names(field_lines(&fields, "PATHWAY")),
        orthology: parse_id_names(field_lines(&fields, "ORTHOLOGY")),
    })
}

fn field_keyword(line: &str) -> String {
    if line.len() >= FIELD_WIDTH {
        line[..FIELD_WIDTH].trim().to_string()
    } else {
        line.trim().to_string()
    }
}

fn field_value(line: &str) -> String {
    if line.len() >= FIELD_WIDTH {
        line[FIELD_WIDTH..].trim_end().to_string()
    } else {
        String::new()
    }
}

fn field_lines<'a>(fields: &'a [(String, Vec<String>)], key: &str) -> &'a [String] {
    fields
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, lines)| lines.as_slice())
        .unwrap_or(&[])
}

fn joined_field(fields: &[(String, Vec<String>)], key: &str) -> Option<String> {
    let text = field_lines(fields, key)
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn parse_names(lines: &[String]) -> Vec<String> {
    let mut names = Vec::new();
    for line in lines {
        for piece in line.split(';') {
            let mut piece = piece.trim();
            if let Some(rest) = piece.strip_prefix("(RefSeq)") {
                piece = rest.trim();
            }
            if !piece.is_empty() {
                names.push(piece.to_string());
            }
        }
    }
    names
}

fn parse_symbols(lines: &[String]) -> Vec<String> {
    let mut symbols = Vec::new();
    for line in lines {
        for piece in line.split(',') {
            let piece = piece.trim();
            if !piece.is_empty() {
                symbols.push(piece.to_string());
            }
        }
    }
    symbols
}

fn parse_id_names(lines: &[String]) -> Vec<Value> {
    let mut out = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match trimmed.split_once(char::is_whitespace) {
            Some((id, name)) => out.push(json!({"id": id, "name": name.trim()})),
            None => out.push(json!({"id": trimmed, "name": ""})),
        }
    }
    out
}

struct KeggRecord {
    entry_id: String,
    entry_type: String,
    name: Vec<String>,
    symbol: Vec<String>,
    definition: Option<String>,
    organism: Option<String>,
    formula: Option<String>,
    pathway: Vec<Value>,
    orthology: Vec<Value>,
}

impl KeggRecord {
    fn matches(&self, requested: &str) -> bool {
        if requested == self.entry_id {
            return true;
        }
        let Some((prefix, local)) = requested.split_once(':') else {
            return false;
        };
        if local != self.entry_id {
            return false;
        }
        match self
            .organism
            .as_deref()
            .and_then(|value| value.split_whitespace().next())
        {
            Some(org) => org == prefix,
            None => true,
        }
    }

    fn into_json(self, requested: &str, raw: Option<String>) -> Value {
        let mut record = json!({
            "requested_id": requested,
            "entry_id": self.entry_id,
            "entry_type": self.entry_type,
            "name": self.name,
            "symbol": self.symbol,
            "definition": self.definition,
            "organism": self.organism,
            "formula": self.formula,
            "pathway": self.pathway,
            "orthology": self.orthology,
            "url": format!("{KEGG_PAGE}/entry/{requested}")
        });
        if let Some(raw) = raw {
            record["raw"] = json!(raw);
        }
        record
    }
}

fn parse_find(text: &str) -> Result<Vec<(String, String)>> {
    if looks_like_html(text.as_bytes()) {
        bail!("KEGG returned HTML instead of text");
    }
    let mut rows = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match line.split_once('\t') {
            Some((id, description)) => {
                rows.push((id.trim().to_string(), description.trim().to_string()))
            }
            None => rows.push((line.trim().to_string(), String::new())),
        }
    }
    Ok(rows)
}

fn exact_symbol_match(description: &str, query: &str) -> bool {
    let symbols = description
        .split_once(';')
        .map(|(left, _)| left)
        .unwrap_or(description);
    let query = query.trim();
    symbols
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .any(|token| token.eq_ignore_ascii_case(query))
}

fn parse_two_column(text: &str) -> Result<Vec<(String, String)>> {
    if looks_like_html(text.as_bytes()) {
        bail!("KEGG returned HTML instead of text");
    }
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        let Some((left, right)) = line.split_once('\t') else {
            bail!("KEGG returned an unusable response");
        };
        let left = left.trim();
        let right = right.trim();
        if left.is_empty() || right.is_empty() {
            continue;
        }
        rows.push((left.to_string(), right.to_string()));
    }
    Ok(rows)
}

fn map_query_id(returned: &str, query_ids: &[String]) -> String {
    if query_ids.iter().any(|id| id == returned) {
        return returned.to_string();
    }
    let local = returned
        .split_once(':')
        .map(|(_, rest)| rest)
        .unwrap_or(returned);
    for id in query_ids {
        if id == local {
            return id.clone();
        }
        let query_local = id
            .split_once(':')
            .map(|(_, rest)| rest)
            .unwrap_or(id.as_str());
        if query_local == local || query_local == returned {
            return id.clone();
        }
    }
    returned.to_string()
}
