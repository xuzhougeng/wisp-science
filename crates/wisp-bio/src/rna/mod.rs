//! Native RNA domain against the Rfam REST API.
//! Independently implemented from:
//!
//! - [Rfam API](https://docs.rfam.org/en/latest/api.html)
//!
//! References reviewed 2026-09-06. Family routes live on `https://rfam.org`.
//! Sequence search is the documented two-step submit/poll flow; this client
//! posts form field `seq` to `/search/sequence` (the shared HTTP helper cannot
//! send the multipart `sequence_file` used by `batch.rfam.org/submit-job`) and
//! negotiates JSON with `?content-type=`. Regions for very large families are
//! refused by Rfam with HTTP 403. Tests use invented records.

#[cfg(test)]
mod tests;

use crate::http::{Response, Source};
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use wisp_llm::ToolSchema;

const RFAM_PUBLIC: &str = "https://rfam.org";
const RFAM: Source = Source("Rfam", Duration::from_millis(500));
const FAMILY_MAX: usize = 64;
const SEARCH_MAX_NT: usize = 10_000;
const DEFAULT_BYTES: u32 = 400_000;
const MAX_BYTES: u32 = 2_000_000;
const DEFAULT_PAGE: u32 = 200;
const MAX_PAGE: u32 = 2_000;
const DEFAULT_HITS: u32 = 50;
const MAX_HITS: u32 = 200;
const MAX_NAMES: usize = 500;
const DEFAULT_WAIT: f64 = 25.0;
const MIN_WAIT: f64 = 1.0;
const MAX_WAIT: f64 = 45.0;
const DEFAULT_POLL: f64 = 1.0;
const MIN_POLL: f64 = 0.5;
const MAX_POLL: f64 = 10.0;

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        (
            "rna",
            ToolSchema::new(
                "accession_to_id",
                "Convert an Rfam accession (RF00001) to its family id through GET /family/{acc}/id. Returns the id, the accession, and the public family URL. Unknown accessions fail rather than returning an empty id.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["accession"],
                    "properties": {
                        "accession": {"type": "string", "minLength": 7, "maxLength": 64,
                            "pattern": "^RF[0-9]{5,8}$"}
                    }
                }),
            ),
        ),
        (
            "rna",
            ToolSchema::new(
                "get_covariance_model",
                "Download the Infernal covariance model for an Rfam family from https://rfam.org/family/{id}/cm. Parsed header fields (NAME, ACC, CLEN, GA, and related CM tags) and size_bytes are always returned. The CM text is omitted when it exceeds max_bytes (default 400000, maximum 2000000); that omission is not an empty model.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["family"],
                    "properties": {
                        "family": {"type": "string", "minLength": 1, "maxLength": 64},
                        "max_bytes": {"type": "integer", "minimum": 1, "maximum": 2000000, "default": 400000}
                    }
                }),
            ),
        ),
        (
            "rna",
            ToolSchema::new(
                "get_family",
                "Retrieve Rfam family metadata by accession (RF00001) or family id from https://rfam.org/family/{id}?content-type=application/json. Returns accession, id, RNA type, seed/full counts, cutoffs, clan and release fields plus source URLs. Unknown families fail rather than returning empty metadata.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["family"],
                    "properties": {
                        "family": {"type": "string", "minLength": 1, "maxLength": 64}
                    }
                }),
            ),
        ),
        (
            "rna",
            ToolSchema::new(
                "get_seed_alignment",
                "Download the Rfam SEED alignment from https://rfam.org/family/{id}/alignment. fmt is stockholm (default), fasta (gapped), pfam, or fastau. Sequence names, counts and size_bytes are always returned. Alignment text is omitted when it exceeds max_bytes (default 400000, maximum 2000000); gzip=1 is never requested.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["family"],
                    "properties": {
                        "family": {"type": "string", "minLength": 1, "maxLength": 64},
                        "fmt": {"type": "string", "enum": ["stockholm", "fasta", "pfam", "fastau"], "default": "stockholm"},
                        "max_bytes": {"type": "integer", "minimum": 1, "maximum": 2000000, "default": 400000}
                    }
                }),
            ),
        ),
        (
            "rna",
            ToolSchema::new(
                "get_sequence_regions",
                "List FULL-alignment sequence regions for an Rfam family from https://rfam.org/family/{id}/regions as a bounded page. Columns are sequence accession, bit score, start, end, description, species and NCBI tax id. Rfam returns HTTP 403 when a family has too many regions — that is an upstream refusal, not an empty hit list. declared_count is Rfam's own count when present; a truncated page is not the complete set.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["family"],
                    "properties": {
                        "family": {"type": "string", "minLength": 1, "maxLength": 64},
                        "max_regions": {"type": "integer", "minimum": 1, "maximum": 2000, "default": 200}
                    }
                }),
            ),
        ),
        (
            "rna",
            ToolSchema::new(
                "get_structure_mapping",
                "Retrieve PDB residue mappings for an Rfam family from https://rfam.org/family/{id}/structures?content-type=application/json. Rows are sorted by pdb_id, chain and coordinates and returned as a bounded page with pdb_ids and RCSB structure URLs. An empty mapping is a family with no 3D coverage, not a transport error.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["family"],
                    "properties": {
                        "family": {"type": "string", "minLength": 1, "maxLength": 64},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 2000, "default": 200}
                    }
                }),
            ),
        ),
        (
            "rna",
            ToolSchema::new(
                "get_tree",
                "Download the Rfam seed phylogenetic tree in NHX/Newick form from https://rfam.org/family/{id}/tree. Returns leaf count, size_bytes and the tree text. The tree is omitted when it exceeds max_bytes (default 400000, maximum 2000000).",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["family"],
                    "properties": {
                        "family": {"type": "string", "minLength": 1, "maxLength": 64},
                        "max_bytes": {"type": "integer", "minimum": 1, "maximum": 2000000, "default": 400000}
                    }
                }),
            ),
        ),
        (
            "rna",
            ToolSchema::new(
                "id_to_accession",
                "Convert an Rfam family id to its accession (RF00001) through GET /family/{id}/acc. Returns the accession, the id, and the public family URL. Unresolved ids fail rather than returning a non-accession string.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["family_id"],
                    "properties": {
                        "family_id": {"type": "string", "minLength": 1, "maxLength": 64}
                    }
                }),
            ),
        ),
        (
            "rna",
            ToolSchema::new(
                "search_sequence",
                "Search one DNA/RNA sequence against the Rfam covariance-model library. Submits POST /search/sequence (form field seq, JSON via content-type) and polls resultURL until HTTP 200 JSON or max_wait_s (1–45s, default 25). Returns a bounded page of hits (family id, accession, coordinates, score, E-value) without alignment blocks. A 5xx on submit means the cmscan backend is unavailable; there is no local fallback. Sequence length is at most 10000 nucleotides after whitespace is stripped.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["sequence"],
                    "properties": {
                        "sequence": {"type": "string", "minLength": 1, "maxLength": 12000},
                        "max_wait_s": {"type": "number", "minimum": 1, "maximum": 45, "default": 25},
                        "poll_interval_s": {"type": "number", "minimum": 0.5, "maximum": 10, "default": 1},
                        "max_hits": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                    }
                }),
            ),
        ),
    ]
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "accession_to_id" => accession_to_id(bio, args).await,
        "get_covariance_model" => get_covariance_model(bio, args).await,
        "get_family" => get_family(bio, args).await,
        "get_seed_alignment" => get_seed_alignment(bio, args).await,
        "get_sequence_regions" => get_sequence_regions(bio, args).await,
        "get_structure_mapping" => get_structure_mapping(bio, args).await,
        "get_tree" => get_tree(bio, args).await,
        "id_to_accession" => id_to_accession(bio, args).await,
        "search_sequence" => search_sequence(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessionQuery {
    accession: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FamilyQuery {
    family: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FamilyIdQuery {
    family_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BytesQuery {
    family: String,
    #[serde(default = "default_bytes")]
    max_bytes: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AlignmentQuery {
    family: String,
    #[serde(default)]
    fmt: AlignmentFormat,
    #[serde(default = "default_bytes")]
    max_bytes: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegionsQuery {
    family: String,
    #[serde(default = "default_page")]
    max_regions: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MappingQuery {
    family: String,
    #[serde(default = "default_page")]
    max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchQuery {
    sequence: String,
    #[serde(default = "default_wait")]
    max_wait_s: f64,
    #[serde(default = "default_poll")]
    poll_interval_s: f64,
    #[serde(default = "default_hits")]
    max_hits: u32,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum AlignmentFormat {
    #[default]
    Stockholm,
    Fasta,
    Pfam,
    Fastau,
}

impl AlignmentFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stockholm => "stockholm",
            Self::Fasta => "fasta",
            Self::Pfam => "pfam",
            Self::Fastau => "fastau",
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::Stockholm => "/alignment",
            Self::Fasta => "/alignment/fasta",
            Self::Pfam => "/alignment/pfam",
            Self::Fastau => "/alignment/fastau",
        }
    }

    fn fasta_names(self) -> bool {
        matches!(self, Self::Fasta | Self::Fastau)
    }
}

fn default_bytes() -> u32 {
    DEFAULT_BYTES
}

fn default_page() -> u32 {
    DEFAULT_PAGE
}

fn default_hits() -> u32 {
    DEFAULT_HITS
}

fn default_wait() -> f64 {
    DEFAULT_WAIT
}

fn default_poll() -> f64 {
    DEFAULT_POLL
}

async fn get_family(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: FamilyQuery =
        serde_json::from_value(args.clone()).context("invalid Rfam family arguments")?;
    let family = require_family(&args.family)?;
    let payload = rfam_json(bio, &family_path(&family, ""), Some(&family)).await?;
    let mut record = family_record(&payload)?;
    record["source"] = json!("Rfam");
    record["source_url"] = json!(RFAM_PUBLIC);
    record["query"] = json!({"family": family});
    let acc = record
        .get("rfam_acc")
        .and_then(Value::as_str)
        .unwrap_or(&family);
    record["family_url"] = json!(public_family_url(acc));
    Ok(record)
}

async fn get_seed_alignment(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: AlignmentQuery =
        serde_json::from_value(args.clone()).context("invalid Rfam alignment arguments")?;
    let family = require_family(&args.family)?;
    let max_bytes = bound_bytes(args.max_bytes)?;
    let text = rfam_text(bio, &family_path(&family, args.fmt.path()), Some(&family)).await?;
    if text.trim().is_empty() {
        bail!("Rfam returned an empty seed alignment for {family}");
    }
    let names = if args.fmt.fasta_names() {
        parse_fasta_seq_names(&text)
    } else {
        parse_stockholm_seq_names(&text)
    };
    let num_sequences = names.len();
    let truncated_names = names.len() > MAX_NAMES;
    let names: Vec<String> = names.into_iter().take(MAX_NAMES).collect();
    let size = text.len();
    let mut result = json!({
        "source": "Rfam",
        "source_url": RFAM_PUBLIC,
        "family_url": public_family_url(&family),
        "query": {"family": family, "fmt": args.fmt.as_str()},
        "family": family,
        "format": args.fmt.as_str(),
        "num_sequences": num_sequences,
        "sequence_names": names,
        "sequence_names_truncated": truncated_names,
        "size_bytes": size,
    });
    attach_text(&mut result, "alignment", text, max_bytes);
    Ok(result)
}

async fn get_covariance_model(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: BytesQuery =
        serde_json::from_value(args.clone()).context("invalid Rfam covariance model arguments")?;
    let family = require_family(&args.family)?;
    let max_bytes = bound_bytes(args.max_bytes)?;
    let text = rfam_text(bio, &family_path(&family, "/cm"), Some(&family)).await?;
    if text.trim().is_empty() {
        bail!("Rfam returned an empty covariance model for {family}");
    }
    let header = parse_cm_header(&text);
    let size = text.len();
    let mut result = json!({
        "source": "Rfam",
        "source_url": RFAM_PUBLIC,
        "family_url": public_family_url(&family),
        "query": {"family": family},
        "family": family,
        "header": header,
        "size_bytes": size,
    });
    attach_text(&mut result, "cm", text, max_bytes);
    Ok(result)
}

async fn get_tree(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: BytesQuery =
        serde_json::from_value(args.clone()).context("invalid Rfam tree arguments")?;
    let family = require_family(&args.family)?;
    let max_bytes = bound_bytes(args.max_bytes)?;
    let text = rfam_text(bio, &family_path(&family, "/tree"), Some(&family)).await?;
    if text.trim().is_empty() {
        bail!("Rfam returned an empty phylogenetic tree for {family}");
    }
    let size = text.len();
    let mut result = json!({
        "source": "Rfam",
        "source_url": RFAM_PUBLIC,
        "family_url": public_family_url(&family),
        "query": {"family": family},
        "family": family,
        "num_leaf_labels": count_newick_leaves(&text),
        "size_bytes": size,
    });
    attach_text(&mut result, "tree", text, max_bytes);
    Ok(result)
}

async fn get_sequence_regions(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: RegionsQuery =
        serde_json::from_value(args.clone()).context("invalid Rfam sequence region arguments")?;
    let family = require_family(&args.family)?;
    let cap = bound_page(args.max_regions, "max_regions")?;
    let response = rfam_get(
        bio,
        &family_path(&family, "/regions"),
        &[("content-type".into(), "text/plain".into())],
    )
    .await?;
    if response.status == StatusCode::FORBIDDEN {
        bail!(
            "Rfam returned HTTP 403 (family {family} has too many sequence regions to list; check num_full with get_family)"
        );
    }
    reject_status(response.status, &format!("Rfam family {family}"))?;
    let text = utf8_text(response)?;
    let parsed = parse_regions(&text)?;
    let total = parsed.regions.len();
    let returned: Vec<Value> = parsed.regions.into_iter().take(cap).collect();
    Ok(json!({
        "source": "Rfam",
        "source_url": RFAM_PUBLIC,
        "family_url": public_family_url(&family),
        "query": {"family": family, "max_regions": cap},
        "family": family,
        "declared_count": parsed.declared_count,
        "num_regions": total,
        "returned": returned.len(),
        "truncated": total > returned.len(),
        "regions": returned,
    }))
}

async fn get_structure_mapping(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: MappingQuery =
        serde_json::from_value(args.clone()).context("invalid Rfam structure mapping arguments")?;
    let family = require_family(&args.family)?;
    let cap = bound_page(args.max_results, "max_results")?;
    let payload = rfam_json(bio, &family_path(&family, "/structures"), Some(&family)).await?;
    let mut rows = structure_rows(&payload)?;
    rows.sort_by(|a, b| mapping_key(a).cmp(&mapping_key(b)));
    let projected: Vec<Value> = rows.iter().map(project_mapping).collect();
    let mut pdb_ids: Vec<String> = projected
        .iter()
        .filter_map(|row| {
            row.get("pdb_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    pdb_ids.sort();
    pdb_ids.dedup();
    let total = projected.len();
    let returned: Vec<Value> = projected.into_iter().take(cap).collect();
    Ok(json!({
        "source": "Rfam",
        "source_url": RFAM_PUBLIC,
        "family_url": public_family_url(&family),
        "query": {"family": family, "max_results": cap},
        "family": family,
        "num_mappings": total,
        "returned": returned.len(),
        "truncated": total > returned.len(),
        "num_pdb_ids": pdb_ids.len(),
        "pdb_ids": pdb_ids,
        "mapping": returned,
    }))
}

async fn accession_to_id(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: AccessionQuery =
        serde_json::from_value(args.clone()).context("invalid Rfam accession arguments")?;
    let accession = require_accession(&args.accession)?;
    let text = rfam_text(bio, &family_path(&accession, "/id"), Some(&accession)).await?;
    let rfam_id = require_family(text.trim())?;
    Ok(json!({
        "source": "Rfam",
        "source_url": RFAM_PUBLIC,
        "family_url": public_family_url(&accession),
        "query": {"accession": accession},
        "accession": accession,
        "rfam_id": rfam_id,
    }))
}

async fn id_to_accession(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: FamilyIdQuery =
        serde_json::from_value(args.clone()).context("invalid Rfam family id arguments")?;
    let family_id = require_family(&args.family_id)?;
    let text = rfam_text(bio, &family_path(&family_id, "/acc"), Some(&family_id)).await?;
    let accession = text.trim();
    if !is_rfam_acc(accession) {
        bail!("Rfam did not resolve {family_id} to an accession (RF00001)");
    }
    Ok(json!({
        "source": "Rfam",
        "source_url": RFAM_PUBLIC,
        "family_url": public_family_url(accession),
        "query": {"family_id": family_id},
        "rfam_id": family_id,
        "accession": accession,
    }))
}

async fn search_sequence(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchQuery =
        serde_json::from_value(args.clone()).context("invalid Rfam sequence search arguments")?;
    let sequence = normalize_sequence(&args.sequence)?;
    let max_wait = bound_seconds(args.max_wait_s, MIN_WAIT, MAX_WAIT, "max_wait_s")?;
    let poll_interval = bound_seconds(args.poll_interval_s, MIN_POLL, MAX_POLL, "poll_interval_s")?;
    let cap = bound_page(args.max_hits, "max_hits")?;
    let base = api_base(bio);
    let submit_url = format!("{base}/search/sequence?content-type=application/json");
    let submit = rfam_send(
        bio,
        Method::POST,
        &submit_url,
        &[("seq".into(), sequence.clone())],
    )
    .await?;
    if submit.status.is_server_error() {
        bail!(
            "Rfam sequence search is unavailable (HTTP {})",
            submit.status.as_u16()
        );
    }
    reject_status(submit.status, "Rfam sequence search")?;
    let payload = parse_json(submit)?;
    if payload.get("hits").is_some() && payload.get("resultURL").is_none() {
        return search_result(&payload, &sequence, None, cap);
    }
    let result_url = payload
        .get("resultURL")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Rfam search response carried no resultURL"))?;
    let poll_url = resolve_result_url(&base, result_url)?;
    let job_id = payload
        .get("jobId")
        .or_else(|| payload.get("job_id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let deadline = Instant::now() + Duration::from_secs_f64(max_wait);
    let mut first = true;
    loop {
        if Instant::now() >= deadline {
            bail!(
                "Rfam sequence search did not finish within {max_wait:.0}s. Retry later or shorten the sequence."
            );
        }
        if !first {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let wait = Duration::from_secs_f64(poll_interval).min(remaining);
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
        }
        first = false;
        let polled = rfam_send(
            bio,
            Method::GET,
            &poll_url,
            &[("content-type".into(), "application/json".into())],
        )
        .await?;
        match classify_poll(polled)? {
            Poll::Ready(body) => {
                return search_result(&body, &sequence, job_id.as_deref(), cap);
            }
            Poll::Pending => {}
        }
    }
}

enum Poll {
    Ready(Value),
    Pending,
}

fn classify_poll(response: Response) -> Result<Poll> {
    match response.status {
        StatusCode::ACCEPTED => Ok(Poll::Pending),
        StatusCode::OK => {
            if looks_like_html_bytes(&response.body) {
                bail!("Rfam returned HTML instead of JSON");
            }
            let text = std::str::from_utf8(&response.body).unwrap_or("").trim();
            if matches!(
                text.to_ascii_uppercase().as_str(),
                "PEND" | "RUN" | "PENDING" | "RUNNING"
            ) {
                return Ok(Poll::Pending);
            }
            let payload: Value =
                serde_json::from_slice(&response.body).context("Rfam returned invalid JSON")?;
            Ok(Poll::Ready(payload))
        }
        StatusCode::TOO_MANY_REQUESTS => bail!("Rfam returned HTTP 429"),
        StatusCode::GONE => bail!("Rfam sequence search job is gone (HTTP 410)"),
        StatusCode::SERVICE_UNAVAILABLE | StatusCode::BAD_GATEWAY => Ok(Poll::Pending),
        other if other.is_server_error() => {
            bail!(
                "Rfam sequence search is unavailable (HTTP {})",
                other.as_u16()
            )
        }
        other => bail!("Rfam returned HTTP {}", other.as_u16()),
    }
}

fn search_result(
    payload: &Value,
    sequence: &str,
    job_id: Option<&str>,
    cap: usize,
) -> Result<Value> {
    let hits = payload.get("hits").unwrap_or(&Value::Null);
    let (families, rows) = flatten_hits(hits)?;
    let job = job_id.map(str::to_string).or_else(|| {
        payload
            .get("jobId")
            .or_else(|| payload.get("job_id"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    let total = rows.len();
    let returned: Vec<Value> = rows.into_iter().take(cap).collect();
    Ok(json!({
        "source": "Rfam",
        "source_url": RFAM_PUBLIC,
        "query": {"sequence_length": sequence.len()},
        "job_id": job,
        "num_hits": total,
        "returned": returned.len(),
        "truncated": total > returned.len(),
        "families": families,
        "hits": returned,
        "search_sequence": sequence,
    }))
}

fn flatten_hits(hits: &Value) -> Result<(Vec<String>, Vec<Value>)> {
    match hits {
        Value::Null => Ok((Vec::new(), Vec::new())),
        Value::Object(map) => {
            let mut families: Vec<String> = map.keys().cloned().collect();
            families.sort();
            let mut rows = Vec::new();
            for family in &families {
                let Some(Value::Array(entries)) = map.get(family) else {
                    continue;
                };
                for hit in entries {
                    if hit.is_object() {
                        rows.push(project_hit(family, hit));
                    }
                }
            }
            Ok((families, rows))
        }
        Value::Array(entries) => {
            let mut rows = Vec::new();
            let mut families = Vec::new();
            for hit in entries {
                if !hit.is_object() {
                    continue;
                }
                let family = hit
                    .get("id")
                    .or_else(|| hit.get("family_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                if !families.iter().any(|name| name == family) {
                    families.push(family.to_string());
                }
                rows.push(project_hit(family, hit));
            }
            families.sort();
            Ok((families, rows))
        }
        _ => bail!("Rfam search hits were not a family map or list"),
    }
}

fn project_hit(family_id: &str, hit: &Value) -> Value {
    let acc = str_field(hit, &["acc", "rfam_acc"]);
    json!({
        "family_id": family_id,
        "id": hit.get("id"),
        "acc": acc,
        "start": hit.get("start"),
        "end": hit.get("end"),
        "strand": hit.get("strand"),
        "gc": hit.get("GC").cloned().or_else(|| hit.get("gc").cloned()),
        "score": hit.get("score"),
        "evalue": hit.get("E").cloned().or_else(|| hit.get("evalue").cloned()),
        "family_url": acc.as_deref().map(public_family_url),
    })
}

async fn rfam_json(bio: &NativeBio, path: &str, family: Option<&str>) -> Result<Value> {
    let response = rfam_get(
        bio,
        path,
        &[("content-type".into(), "application/json".into())],
    )
    .await?;
    reject_status(
        response.status,
        &family
            .map(|id| format!("Rfam family {id}"))
            .unwrap_or_else(|| "Rfam".into()),
    )?;
    parse_json(response)
}

async fn rfam_text(bio: &NativeBio, path: &str, family: Option<&str>) -> Result<String> {
    let response = rfam_get(bio, path, &[("content-type".into(), "text/plain".into())]).await?;
    reject_status(
        response.status,
        &family
            .map(|id| format!("Rfam family {id}"))
            .unwrap_or_else(|| "Rfam".into()),
    )?;
    utf8_text(response)
}

async fn rfam_get(bio: &NativeBio, path: &str, params: &[(String, String)]) -> Result<Response> {
    let url = format!("{}{path}", api_base(bio));
    rfam_send(bio, Method::GET, &url, params).await
}

async fn rfam_send(
    bio: &NativeBio,
    method: Method,
    url: &str,
    params: &[(String, String)],
) -> Result<Response> {
    bio.http().send(RFAM, method, url, params).await
}

fn reject_status(status: StatusCode, context: &str) -> Result<()> {
    if status.is_success() {
        return Ok(());
    }
    let code = status.as_u16();
    match status {
        StatusCode::TOO_MANY_REQUESTS => bail!("Rfam returned HTTP 429"),
        StatusCode::NOT_FOUND => bail!("{context} was not found (HTTP 404)"),
        StatusCode::FORBIDDEN => bail!("Rfam returned HTTP 403"),
        StatusCode::BAD_REQUEST => bail!("Rfam rejected the request (HTTP 400)"),
        _ => bail!("Rfam returned HTTP {code}"),
    }
}

fn parse_json(response: Response) -> Result<Value> {
    if looks_like_html_bytes(&response.body) {
        bail!("Rfam returned HTML instead of JSON");
    }
    serde_json::from_slice(&response.body).context("Rfam returned invalid JSON")
}

fn utf8_text(response: Response) -> Result<String> {
    let text = String::from_utf8(response.body).context("Rfam returned invalid UTF-8")?;
    if looks_like_html(&text) {
        bail!("Rfam returned HTML instead of text");
    }
    Ok(text)
}

fn looks_like_html_bytes(body: &[u8]) -> bool {
    looks_like_html(std::str::from_utf8(body).unwrap_or(""))
}

fn looks_like_html(body: &str) -> bool {
    let prefix: String = body
        .trim_start()
        .chars()
        .take(32)
        .collect::<String>()
        .to_ascii_lowercase();
    prefix.starts_with("<!doctype") || prefix.starts_with("<html")
}

fn attach_text(result: &mut Value, field: &str, text: String, max_bytes: usize) {
    let size = text.len();
    if size > max_bytes {
        result[format!("{field}_omitted")] = json!(format!(
            "{field} is {size} bytes > max_bytes={max_bytes}; metadata and size_bytes are returned — raise max_bytes to include the text"
        ));
    } else {
        result[field] = json!(text);
    }
}

fn family_record(payload: &Value) -> Result<Value> {
    let rfam = payload.get("rfam").unwrap_or(payload);
    if !rfam.is_object() {
        bail!("Rfam family JSON did not include a family object");
    }
    let curation = rfam.get("curation").unwrap_or(&Value::Null);
    let cm = rfam
        .get("cm")
        .or_else(|| rfam.get("cm_details"))
        .unwrap_or(&Value::Null);
    let threshold = cm
        .get("threshold")
        .or_else(|| cm.get("cutoffs"))
        .unwrap_or(&Value::Null);
    let release = rfam.get("release").unwrap_or(&Value::Null);
    let clan = rfam.get("clan").unwrap_or(&Value::Null);
    let acc = str_field(rfam, &["acc", "accession"])
        .ok_or_else(|| anyhow::anyhow!("Rfam family JSON did not include an accession"))?;
    Ok(json!({
        "rfam_acc": acc,
        "rfam_id": str_field(rfam, &["id", "rfam_id"]),
        "description": str_field(rfam, &["description"]),
        "comment": str_field(rfam, &["comment"]),
        "author": str_field(curation, &["author"]),
        "seed_source": str_field(curation, &["seed_source"]),
        "rna_type": str_field(curation, &["type", "rna_type"]),
        "structure_source": str_field(curation, &["structure_source"]),
        "num_seed": json_i64(curation.get("num_seed")),
        "num_full": json_i64(curation.get("num_full")),
        "num_species": json_i64(curation.get("num_species")),
        "gathering_cutoff": json_f64(threshold.get("gathering")).or_else(|| json_f64(curation.get("ga"))),
        "trusted_cutoff": json_f64(threshold.get("trusted")).or_else(|| json_f64(threshold.get("trusted_cutoff"))),
        "noise_cutoff": json_f64(threshold.get("noise")).or_else(|| json_f64(threshold.get("noise_cutoff"))),
        "clan_acc": str_field(clan, &["acc", "clan_acc"]),
        "clan_id": str_field(clan, &["id", "clan_id"]),
        "release_number": str_field(release, &["number"]).or_else(|| json_i64(release.get("number")).map(|n| n.to_string())),
        "release_date": str_field(release, &["date"]),
    }))
}

struct ParsedRegions {
    declared_count: Option<i64>,
    regions: Vec<Value>,
}

const REGION_COLUMNS: [&str; 7] = [
    "sequence_accession",
    "bits_score",
    "region_start",
    "region_end",
    "sequence_description",
    "species",
    "ncbi_tax_id",
];

fn parse_regions(tsv: &str) -> Result<ParsedRegions> {
    if looks_like_html(tsv) {
        bail!("Rfam returned HTML instead of text");
    }
    let mut declared_count = None;
    let mut regions = Vec::new();
    for line in tsv.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('#') {
            let lower = line.to_ascii_lowercase();
            if lower.contains("found") && lower.contains("region") {
                for token in line.split_whitespace() {
                    if let Ok(n) = token.parse::<i64>() {
                        declared_count = Some(n);
                        break;
                    }
                }
            }
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() == 1 && line.split_whitespace().count() >= 4 {
            // some Rfam text dumps use runs of spaces rather than tabs
            let parts: Vec<&str> = line.split_whitespace().collect();
            regions.push(region_row(&parts));
            continue;
        }
        regions.push(region_row(&parts));
    }
    Ok(ParsedRegions {
        declared_count,
        regions,
    })
}

fn region_row(parts: &[&str]) -> Value {
    let mut row = serde_json::Map::new();
    for (idx, key) in REGION_COLUMNS.iter().enumerate() {
        row.insert((*key).into(), json!(parts.get(idx).copied().unwrap_or("")));
    }
    Value::Object(row)
}

fn parse_stockholm_seq_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let name = line.split_whitespace().next().unwrap_or("");
        if name.is_empty() || !seen.insert(name.to_string()) {
            continue;
        }
        names.push(name.to_string());
    }
    names
}

fn parse_fasta_seq_names(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| line.starts_with('>'))
        .map(|line| {
            line[1..]
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string()
        })
        .filter(|name| !name.is_empty())
        .collect()
}

fn parse_cm_header(text: &str) -> Value {
    const WANTED: &[&str] = &[
        "NAME", "ACC", "DESC", "STATES", "NODES", "CLEN", "W", "ALPH", "GA", "TC", "NC",
    ];
    let mut fields = serde_json::Map::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "CM" || trimmed == "//" {
            break;
        }
        let Some((key, rest)) = trimmed.split_once(char::is_whitespace) else {
            continue;
        };
        if !WANTED.contains(&key) || fields.contains_key(key) {
            continue;
        }
        let value = rest.trim();
        let encoded = if matches!(key, "STATES" | "NODES" | "CLEN" | "W") {
            value
                .split_whitespace()
                .next()
                .and_then(|tok| tok.parse::<i64>().ok())
                .map(|n| json!(n))
                .unwrap_or_else(|| json!(value))
        } else {
            json!(value)
        };
        fields.insert(key.to_string(), encoded);
    }
    Value::Object(fields)
}

fn count_newick_leaves(tree: &str) -> usize {
    let stripped = strip_bracket_comments(tree);
    let body = stripped.trim().trim_end_matches(';').trim();
    if body.is_empty() {
        return 0;
    }
    if !body.contains('(') {
        return usize::from(body.contains(':') || body.chars().any(|c| c.is_ascii_alphanumeric()));
    }
    body.chars().filter(|&c| c == ',').count() + 1
}

fn strip_bracket_comments(tree: &str) -> String {
    let mut out = String::with_capacity(tree.len());
    let mut depth = 0usize;
    for c in tree.chars() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

fn structure_rows(payload: &Value) -> Result<Vec<Value>> {
    let rows = match payload {
        Value::Object(map) => map
            .get("mapping")
            .or_else(|| map.get("structures"))
            .cloned()
            .unwrap_or(Value::Array(Vec::new())),
        Value::Array(_) => payload.clone(),
        _ => bail!("Rfam structure mapping JSON was not an object or list"),
    };
    let Some(array) = rows.as_array() else {
        bail!("Rfam structure mapping JSON did not include a mapping list");
    };
    Ok(array
        .iter()
        .filter(|row| row.is_object())
        .cloned()
        .collect())
}

fn project_mapping(row: &Value) -> Value {
    let pdb_id = str_field(row, &["pdb_id", "pdb"]);
    json!({
        "rfam_acc": str_field(row, &["rfam_acc"]),
        "pdb_id": pdb_id,
        "chain": str_field(row, &["chain"]),
        "pdb_start": row.get("pdb_start"),
        "pdb_end": row.get("pdb_end"),
        "cm_start": row.get("cm_start"),
        "cm_end": row.get("cm_end"),
        "seq_start": row.get("seq_start"),
        "seq_end": row.get("seq_end"),
        "bit_score": row.get("bit_score"),
        "evalue_score": row.get("evalue_score"),
        "pdb_url": pdb_id.as_deref().map(|id| format!("https://www.rcsb.org/structure/{}", id.to_ascii_uppercase())),
    })
}

fn mapping_key(row: &Value) -> (String, String, i64, i64, i64) {
    (
        str_field(row, &["pdb_id", "pdb"]).unwrap_or_default(),
        str_field(row, &["chain"]).unwrap_or_default(),
        json_i64(row.get("pdb_start")).unwrap_or(0),
        json_i64(row.get("pdb_end")).unwrap_or(0),
        json_i64(row.get("cm_start")).unwrap_or(0),
    )
}

fn require_family(value: &str) -> Result<String> {
    let family = value.trim();
    if family.is_empty() || family.len() > FAMILY_MAX {
        bail!(
            "family must be an Rfam accession (RF00001) or family id of 1–{FAMILY_MAX} characters"
        );
    }
    if !family
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!(
            "family {family:?} is not a valid Rfam accession or family id (letters, digits, _ or -)"
        );
    }
    Ok(family.to_string())
}

fn require_accession(value: &str) -> Result<String> {
    let accession = require_family(value)?;
    if !is_rfam_acc(&accession) {
        bail!("accession must be an Rfam accession such as RF00001");
    }
    Ok(accession)
}

fn is_rfam_acc(value: &str) -> bool {
    let rest = match value.strip_prefix("RF") {
        Some(rest) => rest,
        None => return false,
    };
    (5..=8).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_digit())
}

fn normalize_sequence(value: &str) -> Result<String> {
    if value.trim_start().starts_with('>') {
        bail!("sequence must be raw nucleotides, not FASTA (omit the header line)");
    }
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if c.is_ascii_whitespace() {
            continue;
        }
        if !is_nucleotide(c) {
            bail!(
                "sequence contains a non-nucleotide character {c:?}; supply DNA/RNA letters only"
            );
        }
        out.push(c);
    }
    if out.is_empty() || out.len() > SEARCH_MAX_NT {
        bail!("sequence must contain 1 to {SEARCH_MAX_NT} nucleotide characters");
    }
    Ok(out)
}

fn is_nucleotide(c: char) -> bool {
    matches!(
        c.to_ascii_uppercase(),
        'A' | 'C'
            | 'G'
            | 'T'
            | 'U'
            | 'N'
            | 'R'
            | 'Y'
            | 'S'
            | 'W'
            | 'K'
            | 'M'
            | 'B'
            | 'D'
            | 'H'
            | 'V'
    )
}

fn bound_bytes(n: u32) -> Result<usize> {
    if !(1..=MAX_BYTES).contains(&n) {
        bail!("max_bytes must be between 1 and {MAX_BYTES}");
    }
    Ok(n as usize)
}

fn bound_page(n: u32, name: &str) -> Result<usize> {
    let max = if name == "max_hits" {
        MAX_HITS
    } else {
        MAX_PAGE
    };
    if n < 1 || n > max {
        bail!("{name} must be between 1 and {max}");
    }
    Ok(n as usize)
}

fn bound_seconds(value: f64, min: f64, max: f64, name: &str) -> Result<f64> {
    if !value.is_finite() || value < min || value > max {
        bail!("{name} must be between {min} and {max}");
    }
    Ok(value)
}

fn resolve_result_url(base: &str, result_url: &str) -> Result<String> {
    let url = result_url.trim();
    if url.is_empty() || url.contains(char::is_whitespace) || url.contains('@') {
        bail!("Rfam search response carried an invalid resultURL");
    }
    let absolute = if url.starts_with('/') {
        format!("{}{url}", base.trim_end_matches('/'))
    } else {
        url.to_string()
    };
    if !(absolute.starts_with("http://") || absolute.starts_with("https://")) {
        bail!("Rfam search response carried an invalid resultURL");
    }
    let host = url_host(&absolute)
        .ok_or_else(|| anyhow::anyhow!("Rfam search response carried an invalid resultURL"))?;
    let base_host = url_host(base).unwrap_or_default();
    let allowed = host.eq_ignore_ascii_case(&base_host)
        || host.eq_ignore_ascii_case("rfam.org")
        || host.eq_ignore_ascii_case("www.rfam.org")
        || host.eq_ignore_ascii_case("batch.rfam.org");
    if !allowed {
        bail!("Rfam search resultURL host is not rfam.org");
    }
    if absolute.contains("..") {
        bail!("Rfam search response carried an invalid resultURL");
    }
    Ok(absolute)
}

fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let hostport = rest.split(['/', '?', '#']).next()?;
    if hostport.contains('@') {
        return None;
    }
    let host = hostport.split(':').next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn api_base(bio: &NativeBio) -> String {
    bio.credential("RFAM_BASE_URL")
        .map(|value| value.trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| RFAM_PUBLIC.to_string())
}

fn family_path(family: &str, suffix: &str) -> String {
    format!("/family/{}{suffix}", path_segment(family))
}

fn public_family_url(family: &str) -> String {
    format!("{RFAM_PUBLIC}/family/{}", path_segment(family))
}

fn path_segment(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn str_field(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        match value.get(*key) {
            Some(Value::String(text)) if !text.is_empty() => return Some(text.clone()),
            Some(Value::Number(number)) => return Some(number.to_string()),
            _ => {}
        }
    }
    None
}

fn json_i64(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Some(Value::String(text)) => text.trim().parse().ok(),
        _ => None,
    }
}

fn json_f64(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(text)) => text.trim().parse().ok(),
        _ => None,
    }
}
