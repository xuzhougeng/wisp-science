//! Frozen PanglaoDB marker table (27 Mar 2020 TSV snapshot). No JSON API.
use crate::http::Source;
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use wisp_llm::ToolSchema;

const SOURCE: &str = "PanglaoDB";
const MARKER_URL: &str = "https://panglaodb.se/markers/PanglaoDB_markers_27_Mar_2020.tsv.gz";
/// sha256 of the gzip bytes as served for the frozen 27 Mar 2020 file.
const PINNED_SHA256: &str = "6779952ad40aa5a124de7bd0e18975c6630bd6006d6b3ef210a916caaa6b53c9";
const PANGLAO: Source = Source(SOURCE, Duration::from_millis(500));
const DEFAULT_ROWS: u32 = 200;
const MAX_ROWS: u32 = 500;
const MAX_SYMBOL: usize = 64;
const MAX_FILTER: usize = 128;
const MAX_TSV: usize = 16 * 1024 * 1024;
const SNAPSHOT_NOTICE: &str = " This is the frozen 27 Mar 2020 PanglaoDB marker snapshot (historical; symbols predate later HGNC/MGI updates). PanglaoDB does not publish a JSON API. Redistribution/commercial terms are not stated on the site; cite Franzén et al. Database (2019) doi:10.1093/database/baz046. The client identifies as wisp-science and does not spoof a browser.";
const HEADER: [&str; 14] = [
    "species",
    "official gene symbol",
    "cell type",
    "nicknames",
    "ubiquitousness index",
    "product description",
    "gene type",
    "canonical marker",
    "germ layer",
    "organ",
    "sensitivity_human",
    "sensitivity_mouse",
    "specificity_human",
    "specificity_mouse",
];

static TABLES: LazyLock<Mutex<HashMap<(String, String), Arc<Vec<MarkerRow>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        tool(
            "panglaodb_cell_types_for_gene",
            "Look up PanglaoDB cell types for which a gene is listed as a marker. gene_symbol is matched case-insensitively against the official gene symbol; include_synonyms also matches pipe-delimited nicknames (the nickname token na is ignored). Each hit is a full marker row plus matched_via (official symbol or synonym). Zero matches is success with total 0.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["gene_symbol"],
                "properties": {
                    "gene_symbol": {"type": "string", "minLength": 1, "maxLength": 64},
                    "include_synonyms": {"type": "boolean", "default": false}
                }
            }),
        ),
        tool(
            "panglaodb_marker_genes",
            "Filter the frozen PanglaoDB cell-type marker table. Optional filters AND together: cell_type and organ (case-insensitive exact match), species Hs or Mm (a row matches when its species field contains that token), sensitivity_min and specificity_max on the species-specific columns (human columns when species is omitted; rows with a missing thresholded value are excluded), and canonical_only (canonical marker == 1). max_rows is 1–500 (default 200). total_matching is the untruncated count; a capped page is not the complete table. Sorted by cell type, official gene symbol, species.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "cell_type": {"type": "string", "minLength": 1, "maxLength": 128},
                    "organ": {"type": "string", "minLength": 1, "maxLength": 128},
                    "species": {
                        "type": "string", "enum": ["Hs", "Mm"],
                        "description": "Hs (human) or Mm (mouse). Rows tagged Mm Hs match either."
                    },
                    "sensitivity_min": {"type": "number"},
                    "specificity_max": {"type": "number"},
                    "canonical_only": {"type": "boolean", "default": false},
                    "max_rows": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200}
                }
            }),
        ),
        tool(
            "panglaodb_options",
            "Enumerate distinct PanglaoDB marker-table values for use as panglaodb_marker_genes filters: species (verbatim, including dirty upstream tokens), organs (organ NA is omitted), cell types, counts, and cell_types_by_organ. Use the returned strings exactly.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {}
            }),
        ),
    ]
}

fn tool(
    name: &'static str,
    description: &'static str,
    parameters: Value,
) -> (&'static str, ToolSchema) {
    (
        "expression",
        ToolSchema::new(name, &format!("{description}{SNAPSHOT_NOTICE}"), parameters),
    )
}

fn default_max_rows() -> u32 {
    DEFAULT_ROWS
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MarkerGenes {
    cell_type: Option<String>,
    organ: Option<String>,
    species: Option<String>,
    sensitivity_min: Option<f64>,
    specificity_max: Option<f64>,
    #[serde(default)]
    canonical_only: bool,
    #[serde(default = "default_max_rows")]
    max_rows: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PanglaoOptions {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CellTypesForGene {
    gene_symbol: String,
    #[serde(default)]
    include_synonyms: bool,
}

#[derive(Clone)]
struct MarkerRow {
    species: String,
    official_gene_symbol: String,
    cell_type: String,
    nicknames: String,
    ubiquitousness_index: Option<f64>,
    product_description: String,
    gene_type: String,
    canonical_marker: String,
    germ_layer: String,
    organ: Option<String>,
    sensitivity_human: Option<f64>,
    sensitivity_mouse: Option<f64>,
    specificity_human: Option<f64>,
    specificity_mouse: Option<f64>,
}

struct Filters {
    cell_type: Option<String>,
    organ: Option<String>,
    species: Option<String>,
    sensitivity_min: Option<f64>,
    specificity_max: Option<f64>,
    canonical_only: bool,
}

pub(super) async fn marker_genes(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: MarkerGenes =
        serde_json::from_value(args.clone()).context("invalid PanglaoDB marker_genes arguments")?;
    let filters = Filters {
        cell_type: optional_filter(args.cell_type.as_deref(), "cell_type")?,
        organ: optional_filter(args.organ.as_deref(), "organ")?,
        species: parse_species(args.species.as_deref())?,
        sensitivity_min: args.sensitivity_min,
        specificity_max: args.specificity_max,
        canonical_only: args.canonical_only,
    };
    let cap = bound_rows(args.max_rows)?;
    let table = load_table(bio).await?;
    let mut matched: Vec<&MarkerRow> = table.iter().filter(|row| row.matches(&filters)).collect();
    matched.sort_by(|a, b| row_sort_key(a).cmp(&row_sort_key(b)));
    let total = matched.len();
    let markers: Vec<Value> = matched
        .into_iter()
        .take(cap)
        .map(MarkerRow::to_json)
        .collect();
    Ok(json!({
        "source": SOURCE,
        "source_url": MARKER_URL,
        "total_matching": total,
        "returned": markers.len(),
        "truncated": markers.len() < total,
        "markers": markers
    }))
}

pub(super) async fn options(bio: &NativeBio, args: &Value) -> Result<Value> {
    let _: PanglaoOptions =
        serde_json::from_value(args.clone()).context("invalid PanglaoDB options arguments")?;
    let table = load_table(bio).await?;
    let mut species = BTreeSet::new();
    let mut organs = BTreeSet::new();
    let mut cell_types = BTreeSet::new();
    let mut by_organ: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in table.iter() {
        if !row.species.is_empty() {
            species.insert(row.species.clone());
        }
        if !row.cell_type.is_empty() {
            cell_types.insert(row.cell_type.clone());
        }
        if let Some(organ) = &row.organ {
            organs.insert(organ.clone());
            if !row.cell_type.is_empty() {
                by_organ
                    .entry(organ.clone())
                    .or_default()
                    .insert(row.cell_type.clone());
            }
        }
    }
    let organs: Vec<String> = organs.into_iter().collect();
    let cell_types: Vec<String> = cell_types.into_iter().collect();
    let n_organs = organs.len();
    let n_cell_types = cell_types.len();
    let cell_types_by_organ: BTreeMap<String, Vec<String>> = by_organ
        .into_iter()
        .map(|(organ, types)| (organ, types.into_iter().collect()))
        .collect();
    Ok(json!({
        "source": SOURCE,
        "source_url": MARKER_URL,
        "species": species.into_iter().collect::<Vec<_>>(),
        "organs": organs,
        "cell_types": cell_types,
        "n_organs": n_organs,
        "n_cell_types": n_cell_types,
        "cell_types_by_organ": cell_types_by_organ
    }))
}

pub(super) async fn cell_types_for_gene(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: CellTypesForGene = serde_json::from_value(args.clone())
        .context("invalid PanglaoDB cell_types_for_gene arguments")?;
    let gene = require_text(&args.gene_symbol, "gene_symbol", MAX_SYMBOL)?;
    let table = load_table(bio).await?;
    let mut matches: Vec<(&MarkerRow, &'static str)> = Vec::new();
    for row in table.iter() {
        if row.official_gene_symbol.eq_ignore_ascii_case(&gene) {
            matches.push((row, "official symbol"));
        } else if args.include_synonyms && synonym_match(&row.nicknames, &gene) {
            matches.push((row, "synonym"));
        }
    }
    matches.sort_by(|a, b| row_sort_key(a.0).cmp(&row_sort_key(b.0)));
    let records: Vec<Value> = matches
        .into_iter()
        .map(|(row, via)| {
            let mut record = row.to_json();
            record["matched_via"] = json!(via);
            record
        })
        .collect();
    Ok(json!({
        "source": SOURCE,
        "source_url": MARKER_URL,
        "total": records.len(),
        "matches": records
    }))
}

async fn load_table(bio: &NativeBio) -> Result<Arc<Vec<MarkerRow>>> {
    let url = marker_url(bio);
    let expected = expected_sha256(bio);
    if let Some(digest) = &expected {
        if let Some(table) = cache_get(&url, digest) {
            return Ok(table);
        }
    }
    let gzip = download_markers(bio, &url).await?;
    let actual = sha256_hex(&gzip);
    if let Some(expected) = &expected {
        if actual != *expected {
            bail!(
                "PanglaoDB marker file checksum mismatch (expected {expected}, got {actual}); refusing to parse"
            );
        }
    }
    if let Some(table) = cache_get(&url, &actual) {
        return Ok(table);
    }
    let rows = parse_markers(&gzip)?;
    let table = Arc::new(rows);
    cache_put(url, actual, table.clone());
    Ok(table)
}

async fn download_markers(bio: &NativeBio, url: &str) -> Result<Vec<u8>> {
    let response = bio.http().send(PANGLAO, Method::GET, url, &[]).await?;
    if response.status == StatusCode::FORBIDDEN {
        bail!(
            "PanglaoDB returned HTTP 403; the server may reject the wisp-science User-Agent. This client does not spoof a browser."
        );
    }
    response.check()?;
    Ok(response.body)
}

fn parse_markers(gzip: &[u8]) -> Result<Vec<MarkerRow>> {
    let text = decode_gzip(gzip)?;
    let mut lines = text.lines();
    let header = lines.next().context("PanglaoDB marker file was empty")?;
    if header.split('\t').collect::<Vec<_>>() != HEADER {
        bail!("PanglaoDB marker file had an unexpected header");
    }
    let mut rows = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != HEADER.len() {
            bail!("PanglaoDB marker file had a malformed row");
        }
        rows.push(parse_row(&fields)?);
    }
    Ok(rows)
}

fn parse_row(fields: &[&str]) -> Result<MarkerRow> {
    Ok(MarkerRow {
        species: fields[0].trim().to_string(),
        official_gene_symbol: fields[1].trim().to_string(),
        cell_type: fields[2].trim().to_string(),
        nicknames: fields[3].trim().to_string(),
        ubiquitousness_index: parse_number(fields[4], "ubiquitousness index")?,
        product_description: fields[5].trim().to_string(),
        gene_type: fields[6].trim().to_string(),
        canonical_marker: fields[7].trim().to_string(),
        germ_layer: fields[8].trim().to_string(),
        organ: parse_organ(fields[9]),
        sensitivity_human: parse_number(fields[10], "sensitivity_human")?,
        sensitivity_mouse: parse_number(fields[11], "sensitivity_mouse")?,
        specificity_human: parse_number(fields[12], "specificity_human")?,
        specificity_mouse: parse_number(fields[13], "specificity_mouse")?,
    })
}

fn parse_organ(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value == "NA" {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_number(value: &str, what: &str) -> Result<Option<f64>> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("NA") || value.eq_ignore_ascii_case("NaN") {
        return Ok(None);
    }
    let number: f64 = value
        .parse()
        .with_context(|| format!("PanglaoDB {what} {value:?} was not a number"))?;
    if !number.is_finite() {
        return Ok(None);
    }
    Ok(Some(number))
}

fn decode_gzip(bytes: &[u8]) -> Result<String> {
    let mut decoder = GzDecoder::new(bytes);
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = decoder
            .read(&mut chunk)
            .context("PanglaoDB marker file was not valid gzip")?;
        if n == 0 {
            break;
        }
        if buf.len().saturating_add(n) > MAX_TSV {
            bail!("PanglaoDB marker table exceeded the in-memory size limit");
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8(buf).context("PanglaoDB marker file was not valid UTF-8")
}

impl MarkerRow {
    fn matches(&self, filters: &Filters) -> bool {
        if let Some(cell_type) = &filters.cell_type {
            if !self.cell_type.eq_ignore_ascii_case(cell_type) {
                return false;
            }
        }
        if let Some(organ) = &filters.organ {
            if !self
                .organ
                .as_deref()
                .unwrap_or("")
                .eq_ignore_ascii_case(organ)
            {
                return false;
            }
        }
        if let Some(species) = &filters.species {
            if !self
                .species
                .split_whitespace()
                .any(|token| token == species)
            {
                return false;
            }
        }
        if let Some(min) = filters.sensitivity_min {
            match self.sensitivity_for(filters.species.as_deref()) {
                Some(value) if value >= min => {}
                _ => return false,
            }
        }
        if let Some(max) = filters.specificity_max {
            match self.specificity_for(filters.species.as_deref()) {
                Some(value) if value <= max => {}
                _ => return false,
            }
        }
        if filters.canonical_only && self.canonical_marker != "1" {
            return false;
        }
        true
    }

    fn sensitivity_for(&self, species: Option<&str>) -> Option<f64> {
        match species {
            Some("Mm") => self.sensitivity_mouse,
            _ => self.sensitivity_human,
        }
    }

    fn specificity_for(&self, species: Option<&str>) -> Option<f64> {
        match species {
            Some("Mm") => self.specificity_mouse,
            _ => self.specificity_human,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "species": self.species,
            "official_gene_symbol": self.official_gene_symbol,
            "cell_type": self.cell_type,
            "nicknames": self.nicknames,
            "ubiquitousness_index": self.ubiquitousness_index,
            "product_description": self.product_description,
            "gene_type": self.gene_type,
            "canonical_marker": self.canonical_marker,
            "germ_layer": self.germ_layer,
            "organ": self.organ,
            "sensitivity_human": self.sensitivity_human,
            "sensitivity_mouse": self.sensitivity_mouse,
            "specificity_human": self.specificity_human,
            "specificity_mouse": self.specificity_mouse
        })
    }
}

fn row_sort_key(row: &MarkerRow) -> (&str, &str, &str) {
    (&row.cell_type, &row.official_gene_symbol, &row.species)
}

fn synonym_match(nicknames: &str, gene: &str) -> bool {
    if gene.eq_ignore_ascii_case("na") {
        return false;
    }
    nicknames.split('|').any(|token| {
        let token = token.trim();
        !token.eq_ignore_ascii_case("na") && token.eq_ignore_ascii_case(gene)
    })
}

fn parse_species(value: Option<&str>) -> Result<Option<String>> {
    match value.map(str::trim).filter(|text| !text.is_empty()) {
        None => Ok(None),
        Some("Hs") => Ok(Some("Hs".into())),
        Some("Mm") => Ok(Some("Mm".into())),
        Some(other) => bail!("species must be Hs or Mm, not {other:?}"),
    }
}

fn optional_filter(value: Option<&str>, what: &str) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(text) if text.trim().is_empty() => Ok(None),
        Some(text) => Ok(Some(require_text(text, what, MAX_FILTER)?)),
    }
}

fn require_text(value: &str, what: &str, max: usize) -> Result<String> {
    let text = value.trim();
    if text.is_empty() || text.len() > max {
        bail!("{what} must contain 1 to {max} characters");
    }
    Ok(text.to_string())
}

fn bound_rows(n: u32) -> Result<usize> {
    if !(1..=MAX_ROWS).contains(&n) {
        bail!("max_rows must be between 1 and {MAX_ROWS}");
    }
    Ok(n as usize)
}

fn marker_url(bio: &NativeBio) -> String {
    bio.credential("PANGLAODB_MARKER_URL")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(MARKER_URL)
        .to_string()
}

fn expected_sha256(bio: &NativeBio) -> Option<String> {
    if bio
        .credential("PANGLAODB_VERIFY_CHECKSUM")
        .is_some_and(|value| value.trim() == "0")
    {
        return None;
    }
    Some(
        bio.credential("PANGLAODB_SHA256")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(PINNED_SHA256)
            .to_ascii_lowercase(),
    )
}

fn cache_get(url: &str, digest: &str) -> Option<Arc<Vec<MarkerRow>>> {
    TABLES
        .lock()
        .unwrap()
        .get(&(url.to_string(), digest.to_string()))
        .cloned()
}

fn cache_put(url: String, digest: String, table: Arc<Vec<MarkerRow>>) {
    TABLES.lock().unwrap().insert((url, digest), table);
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for &byte in digest.as_slice() {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
