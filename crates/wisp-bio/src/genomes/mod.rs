//! Native `genomes` domain against Ensembl REST and the UCSC Genome Browser API.
//! Independently implemented from:
//!
//! - [Ensembl REST](https://rest.ensembl.org/)
//! - [Ensembl REST user guide](https://github.com/Ensembl/ensembl-rest/wiki)
//!   (rate limits, JSON default, HTTP 400 for unknown IDs)
//! - [UCSC REST API](https://genome.ucsc.edu/goldenPath/help/api.html)
//!
//! References reviewed 2026-09-06. Both APIs are keyless GETs. Ensembl uses
//! 1-based inclusive coordinates without a `chr` prefix; UCSC uses 0-based
//! half-open coordinates and `chr`-prefixed names on human assemblies. Tests
//! use invented records.

#[cfg(test)]
mod tests;

use crate::http::Source;
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::time::Duration;
use wisp_llm::ToolSchema;

const ENSEMBL_REST: &str = "https://rest.ensembl.org";
const UCSC_API: &str = "https://api.genome.ucsc.edu";
const ENSEMBL_BROWSER: &str = "https://www.ensembl.org";
const UCSC_BROWSER: &str = "https://genome.ucsc.edu/cgi-bin/hgTracks";
const ENSEMBL: Source = Source("Ensembl REST", Duration::from_millis(340));
const UCSC: Source = Source("UCSC Genome Browser", Duration::from_millis(500));

const DEFAULT_SPECIES: &str = "homo_sapiens";
const DEFAULT_GENOME: &str = "hg38";
const MAX_ID: usize = 64;
const MAX_REGION: usize = 128;
const MAX_ALLELE: usize = 1024;
const MAX_FILTER: usize = 128;
const MAX_CONSEQUENCES: u32 = 200;
const MAX_HOMOLOGIES: u32 = 500;
const MAX_FEATURES: u32 = 2_000;
const MAX_BYTES: u32 = 400_000;
const MAX_TRACKS: u32 = 1_000;
const MAX_ROWS: u32 = 10_000;
const MAX_VALUES: u32 = 10_000;
const MAX_CHROMS: u32 = 500;
const MAX_CONSERVATION_SPAN: i64 = 100_000;
const MAX_COORD: i64 = 3_000_000_000;

const SEQ_TYPES: &[&str] = &["genomic", "cdna", "cds", "protein"];
const HOMOLOGY_TYPES: &[&str] = &["orthologues", "paralogues", "projections"];
const OVERLAP_FEATURES: &[&str] = &[
    "gene",
    "transcript",
    "exon",
    "cds",
    "regulatory",
    "motif",
    "repeat",
    "variation",
    "structural_variation",
    "band",
    "simple",
    "misc",
];
const IMPACT_RANK: &[(&str, u8)] = &[("HIGH", 0), ("MODERATE", 1), ("LOW", 2), ("MODIFIER", 3)];

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        (
            "genomes",
            ToolSchema::new(
                "ensembl_homology",
                "Retrieve Ensembl Compara orthologues, paralogues or projections for one gene as condensed rows (no alignments). Pass exactly one of gene_symbol or gene_id. Symbols are resolved with GET /lookup/symbol then queried with GET /homology/id/:species/:id (the /homology/symbol route is not used). n_total is the complete Compara set; homologies_truncated flags the output cap. Coordinates are Ensembl 1-based inclusive.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "gene_symbol": {"type": "string", "minLength": 1, "maxLength": 64},
                        "gene_id": {"type": "string", "minLength": 1, "maxLength": 64},
                        "homology_type": {"type": "string", "enum": ["orthologues", "paralogues", "projections"], "default": "orthologues"},
                        "target_species": {"type": "string", "minLength": 1, "maxLength": 64},
                        "target_taxon": {"type": "integer", "minimum": 1, "maximum": 99999999},
                        "species": {"type": "string", "minLength": 1, "maxLength": 64, "default": "homo_sapiens"},
                        "max_homologies": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200}
                    }
                }),
            ),
        ),
        (
            "genomes",
            ToolSchema::new(
                "ensembl_lookup",
                "Look up one Ensembl gene, transcript or protein. Stable IDs (ENSG/ENST/ENSP/ENSE/ENSR, including species-prefixed and versioned forms, or LRG_N) use GET /lookup/id/:id; other queries use GET /lookup/symbol/:species/:symbol. expand=true includes child transcripts/exons. found is false when Ensembl returns HTTP 400/404. The record uses 1-based inclusive coordinates and includes assembly_name (GRCh38 for current human).",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["query"],
                    "properties": {
                        "query": {"type": "string", "minLength": 1, "maxLength": 64},
                        "species": {"type": "string", "minLength": 1, "maxLength": 64, "default": "homo_sapiens"},
                        "expand": {"type": "boolean", "default": false}
                    }
                }),
            ),
        ),
        (
            "genomes",
            ToolSchema::new(
                "ensembl_overlap_region",
                "List Ensembl features overlapping a genomic region via GET /overlap/region/:species/:region. Region is chrom:start-end, 1-based inclusive (GRCh38 for current human). Ensembl rejects spans above 5 Mb. n_total is the complete overlap; features_truncated flags the output cap. feature selects gene, transcript, exon, cds, regulatory, motif, repeat, variation, structural_variation, band, simple or misc.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["region"],
                    "properties": {
                        "region": {"type": "string", "minLength": 3, "maxLength": 128},
                        "feature": {"type": "string", "enum": ["gene", "transcript", "exon", "cds", "regulatory", "motif", "repeat", "variation", "structural_variation", "band", "simple", "misc"], "default": "gene"},
                        "species": {"type": "string", "minLength": 1, "maxLength": 64, "default": "homo_sapiens"},
                        "max_features": {"type": "integer", "minimum": 1, "maximum": 2000, "default": 500}
                    }
                }),
            ),
        ),
        (
            "genomes",
            ToolSchema::new(
                "ensembl_sequence",
                "Fetch sequence from Ensembl. Pass exactly one of stable_id (GET /sequence/id/:id) or region (GET /sequence/region/:species/:region). seq_type applies to the ID route: genomic, cdna, cds or protein (protein only for ENST/ENSP). Region queries always return genomic DNA. Sequences larger than max_bytes omit seq but keep length and sha256. found is false for unknown stable IDs.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "stable_id": {"type": "string", "minLength": 1, "maxLength": 64},
                        "region": {"type": "string", "minLength": 3, "maxLength": 128},
                        "species": {"type": "string", "minLength": 1, "maxLength": 64, "default": "homo_sapiens"},
                        "seq_type": {"type": "string", "enum": ["genomic", "cdna", "cds", "protein"], "default": "genomic"},
                        "max_bytes": {"type": "integer", "minimum": 1, "maximum": 400000, "default": 400000}
                    }
                }),
            ),
        ),
        (
            "genomes",
            ToolSchema::new(
                "ensembl_vep_variant",
                "Predict variant consequences with Ensembl VEP. Pass exactly one of variant_id (GET /vep/:species/id/:id — rsID, COSMIC or HGMD) or region plus allele (GET /vep/:species/region/:region/:allele, 1-based inclusive chrom:start-end). Transcript consequences are ordered HIGH > MODERATE > LOW > MODIFIER and capped; n_transcript_consequences is the complete count. Unknown identifiers fail with the HTTP status.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "variant_id": {"type": "string", "minLength": 1, "maxLength": 64},
                        "region": {"type": "string", "minLength": 3, "maxLength": 128},
                        "allele": {"type": "string", "minLength": 1, "maxLength": 1024},
                        "species": {"type": "string", "minLength": 1, "maxLength": 64, "default": "homo_sapiens"},
                        "max_consequences": {"type": "integer", "minimum": 1, "maximum": 200, "default": 25}
                    }
                }),
            ),
        ),
        (
            "genomes",
            ToolSchema::new(
                "ensembl_xrefs",
                "List external cross-references for one Ensembl stable ID via GET /xrefs/id/:id (HGNC, EntrezGene, UniProt, OMIM, RefSeq, and others). external_db, when set, is forwarded as an exact upstream database-name filter. The complete list is returned, sorted by dbname then primary_id. Unknown IDs return n_xrefs: 0.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["stable_id"],
                    "properties": {
                        "stable_id": {"type": "string", "minLength": 1, "maxLength": 64},
                        "external_db": {"type": "string", "minLength": 1, "maxLength": 64}
                    }
                }),
            ),
        ),
        (
            "genomes",
            ToolSchema::new(
                "ucsc_chrom_sizes",
                "List chromosome and contig names with sizes for a UCSC assembly via GET /list/chromosomes. chrom_count is the assembly-wide total from the API; n_total is the post-filter count and chroms_truncated flags the output cap. Rows are sorted largest-first so primary chromosomes lead on human. Coordinates are not returned; sizes are in base pairs.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "genome": {"type": "string", "minLength": 1, "maxLength": 32, "default": "hg38"},
                        "filter_text": {"type": "string", "minLength": 1, "maxLength": 128},
                        "max_chroms": {"type": "integer", "minimum": 1, "maximum": 500, "default": 100}
                    }
                }),
            ),
        ),
        (
            "genomes",
            ToolSchema::new(
                "ucsc_conservation",
                "Summarise per-base conservation for a UCSC wiggle/bigWig track (default phyloP100way) via GET /getData/track. Coordinates are 0-based half-open and chr-prefixed. Span is capped at 100000 bp. Stats are span-weighted and clipped to the window; uncovered bases are omitted from the mean rather than scored as zero. A truncated listing or a non-score track is an error. include_values adds the clipped per-base rows used for the summary.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["chrom", "start", "end"],
                    "properties": {
                        "chrom": {"type": "string", "minLength": 1, "maxLength": 64},
                        "start": {"type": "integer", "minimum": 0, "maximum": 3000000000_i64},
                        "end": {"type": "integer", "minimum": 1, "maximum": 3000000000_i64},
                        "genome": {"type": "string", "minLength": 1, "maxLength": 32, "default": "hg38"},
                        "track": {"type": "string", "minLength": 1, "maxLength": 128, "default": "phyloP100way"},
                        "include_values": {"type": "boolean", "default": false},
                        "max_values": {"type": "integer", "minimum": 1, "maximum": 10000, "default": 2000}
                    }
                }),
            ),
        ),
        (
            "genomes",
            ToolSchema::new(
                "ucsc_list_tracks",
                "List leaf tracks for a UCSC assembly via GET /list/tracks?trackLeavesOnly=1. filter_text is a case-insensitive substring over track name, shortLabel and longLabel applied after download. The full listing is subject to the 4 MiB retrieval bound, so a filter is recommended on large assemblies. n_total is the complete match count; tracks_truncated flags the output cap. Use track with ucsc_track_data.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "genome": {"type": "string", "minLength": 1, "maxLength": 32, "default": "hg38"},
                        "filter_text": {"type": "string", "minLength": 1, "maxLength": 128},
                        "max_tracks": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 200}
                    }
                }),
            ),
        ),
        (
            "genomes",
            ToolSchema::new(
                "ucsc_tfbs_clusters",
                "ENCODE transcription-factor binding-site clusters overlapping a region via GET /getData/track. hg38 uses encRegTfbsClustered; hg19 uses wgEncodeRegTfbsClusteredV3. Other assemblies are rejected. Coordinates are 0-based half-open and chr-prefixed. truncated reflects the API maxItemsLimit flag. factors lists distinct TF names; score is cluster strength (0–1000).",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["chrom", "start", "end"],
                    "properties": {
                        "chrom": {"type": "string", "minLength": 1, "maxLength": 64},
                        "start": {"type": "integer", "minimum": 0, "maximum": 3000000000_i64},
                        "end": {"type": "integer", "minimum": 1, "maximum": 3000000000_i64},
                        "genome": {"type": "string", "minLength": 1, "maxLength": 32, "default": "hg38"},
                        "max_rows": {"type": "integer", "minimum": 1, "maximum": 10000, "default": 1000}
                    }
                }),
            ),
        ),
        (
            "genomes",
            ToolSchema::new(
                "ucsc_track_data",
                "Fetch raw rows for any UCSC track in a region via GET /getData/track. Coordinates are 0-based half-open and chr-prefixed. max_rows is sent as maxItemsOutput. truncated reflects the API maxItemsLimit flag (HTTP 206 is still success). BED-like rows keep chrom/chromStart/chromEnd; wiggle rows keep start/end/value. data_download_url is echoed when the API supplies it for huge tracks.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["track", "chrom", "start", "end"],
                    "properties": {
                        "track": {"type": "string", "minLength": 1, "maxLength": 128},
                        "chrom": {"type": "string", "minLength": 1, "maxLength": 64},
                        "start": {"type": "integer", "minimum": 0, "maximum": 3000000000_i64},
                        "end": {"type": "integer", "minimum": 1, "maximum": 3000000000_i64},
                        "genome": {"type": "string", "minLength": 1, "maxLength": 32, "default": "hg38"},
                        "max_rows": {"type": "integer", "minimum": 1, "maximum": 10000, "default": 1000}
                    }
                }),
            ),
        ),
    ]
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "ensembl_homology" => ensembl_homology(bio, args).await,
        "ensembl_lookup" => ensembl_lookup(bio, args).await,
        "ensembl_overlap_region" => ensembl_overlap_region(bio, args).await,
        "ensembl_sequence" => ensembl_sequence(bio, args).await,
        "ensembl_vep_variant" => ensembl_vep_variant(bio, args).await,
        "ensembl_xrefs" => ensembl_xrefs(bio, args).await,
        "ucsc_chrom_sizes" => ucsc_chrom_sizes(bio, args).await,
        "ucsc_conservation" => ucsc_conservation(bio, args).await,
        "ucsc_list_tracks" => ucsc_list_tracks(bio, args).await,
        "ucsc_tfbs_clusters" => ucsc_tfbs_clusters(bio, args).await,
        "ucsc_track_data" => ucsc_track_data(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HomologyArgs {
    gene_symbol: Option<String>,
    gene_id: Option<String>,
    #[serde(default = "default_homology_type")]
    homology_type: String,
    target_species: Option<String>,
    target_taxon: Option<i64>,
    #[serde(default = "default_species")]
    species: String,
    #[serde(default = "default_max_homologies")]
    max_homologies: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LookupArgs {
    query: String,
    #[serde(default = "default_species")]
    species: String,
    #[serde(default)]
    expand: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OverlapArgs {
    region: String,
    #[serde(default = "default_feature")]
    feature: String,
    #[serde(default = "default_species")]
    species: String,
    #[serde(default = "default_max_features")]
    max_features: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SequenceArgs {
    stable_id: Option<String>,
    region: Option<String>,
    #[serde(default = "default_species")]
    species: String,
    #[serde(default = "default_seq_type")]
    seq_type: String,
    #[serde(default = "default_max_bytes")]
    max_bytes: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VepArgs {
    variant_id: Option<String>,
    region: Option<String>,
    allele: Option<String>,
    #[serde(default = "default_species")]
    species: String,
    #[serde(default = "default_max_consequences")]
    max_consequences: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct XrefsArgs {
    stable_id: String,
    external_db: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChromSizesArgs {
    #[serde(default = "default_genome")]
    genome: String,
    filter_text: Option<String>,
    #[serde(default = "default_max_chroms")]
    max_chroms: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConservationArgs {
    chrom: String,
    start: i64,
    end: i64,
    #[serde(default = "default_genome")]
    genome: String,
    #[serde(default = "default_phylo_track")]
    track: String,
    #[serde(default)]
    include_values: bool,
    #[serde(default = "default_max_values")]
    max_values: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListTracksArgs {
    #[serde(default = "default_genome")]
    genome: String,
    filter_text: Option<String>,
    #[serde(default = "default_max_tracks")]
    max_tracks: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TfbsArgs {
    chrom: String,
    start: i64,
    end: i64,
    #[serde(default = "default_genome")]
    genome: String,
    #[serde(default = "default_max_rows")]
    max_rows: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackDataArgs {
    track: String,
    chrom: String,
    start: i64,
    end: i64,
    #[serde(default = "default_genome")]
    genome: String,
    #[serde(default = "default_max_rows")]
    max_rows: u32,
}

fn default_species() -> String {
    DEFAULT_SPECIES.into()
}
fn default_genome() -> String {
    DEFAULT_GENOME.into()
}
fn default_homology_type() -> String {
    "orthologues".into()
}
fn default_feature() -> String {
    "gene".into()
}
fn default_seq_type() -> String {
    "genomic".into()
}
fn default_phylo_track() -> String {
    "phyloP100way".into()
}
fn default_max_homologies() -> u32 {
    200
}
fn default_max_features() -> u32 {
    500
}
fn default_max_bytes() -> u32 {
    MAX_BYTES
}
fn default_max_consequences() -> u32 {
    25
}
fn default_max_chroms() -> u32 {
    100
}
fn default_max_tracks() -> u32 {
    200
}
fn default_max_rows() -> u32 {
    1000
}
fn default_max_values() -> u32 {
    2000
}

async fn ensembl_lookup(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: LookupArgs =
        serde_json::from_value(args.clone()).context("invalid Ensembl lookup arguments")?;
    let query = ident(&args.query, "query", MAX_ID, &['-', '.'])?;
    let species = species_name(&args.species)?;
    let record = if looks_like_stable_id(&query) {
        ensembl_optional(
            bio,
            &format!("/lookup/id/{}", path_segment(&query)),
            &[("expand".into(), i32::from(args.expand).to_string())],
        )
        .await?
    } else {
        ensembl_optional(
            bio,
            &format!(
                "/lookup/symbol/{}/{}",
                path_segment(&species),
                path_segment(&query)
            ),
            &[("expand".into(), i32::from(args.expand).to_string())],
        )
        .await?
    };
    let url = record
        .as_ref()
        .and_then(|rec| ensembl_browser_url(&species, rec));
    Ok(json!({
        "source": "Ensembl REST",
        "source_url": ENSEMBL_REST,
        "found": record.is_some(),
        "query": query,
        "species": species,
        "url": url,
        "record": record,
    }))
}

async fn ensembl_xrefs(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: XrefsArgs =
        serde_json::from_value(args.clone()).context("invalid Ensembl xref arguments")?;
    let stable_id = ident(&args.stable_id, "stable_id", MAX_ID, &['.', '-'])?;
    let external_db = optional_ident(&args.external_db, "external_db", MAX_ID, &['_'])?;
    let mut params = Vec::new();
    if let Some(db) = &external_db {
        params.push(("external_db".into(), db.clone()));
    }
    let raw = ensembl_optional(
        bio,
        &format!("/xrefs/id/{}", path_segment(&stable_id)),
        &params,
    )
    .await?;
    let mut rows = match raw {
        None => Vec::new(),
        Some(Value::Array(rows)) => rows,
        Some(_) => bail!("Ensembl REST /xrefs/id returned an unrecognized shape"),
    };
    rows.sort_by(|a, b| {
        (
            text(a, "dbname").unwrap_or_default(),
            text(a, "primary_id").unwrap_or_default(),
        )
            .cmp(&(
                text(b, "dbname").unwrap_or_default(),
                text(b, "primary_id").unwrap_or_default(),
            ))
    });
    Ok(json!({
        "source": "Ensembl REST",
        "source_url": ENSEMBL_REST,
        "stable_id": stable_id,
        "external_db": external_db,
        "n_xrefs": rows.len(),
        "xrefs": rows,
    }))
}

async fn ensembl_vep_variant(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: VepArgs =
        serde_json::from_value(args.clone()).context("invalid Ensembl VEP arguments")?;
    let variant_id = optional_ident(&args.variant_id, "variant_id", MAX_ID, &['.', '-', '_'])?;
    let region = optional_region(&args.region)?;
    let allele = optional_allele(&args.allele)?;
    if variant_id.is_none() == (region.is_none() && allele.is_none()) {
        bail!("pass exactly one of variant_id, or region + allele");
    }
    let species = species_name(&args.species)?;
    let cap = require_cap(args.max_consequences, "max_consequences", MAX_CONSEQUENCES)?;
    let (path, query) = if let Some(id) = &variant_id {
        (
            format!("/vep/{}/id/{}", path_segment(&species), path_segment(id)),
            json!({"variant_id": id, "species": species}),
        )
    } else {
        let region = region.context("the region route needs both region and allele")?;
        let allele = allele.context("the region route needs both region and allele")?;
        (
            format!(
                "/vep/{}/region/{}/{}",
                path_segment(&species),
                path_segment(&region),
                path_segment(&allele)
            ),
            json!({"region": region, "allele": allele, "species": species}),
        )
    };
    let raw = ensembl_json(bio, &path, &[]).await?;
    let results = vep_results(&raw, cap)?;
    Ok(json!({
        "source": "Ensembl REST",
        "source_url": ENSEMBL_REST,
        "query": query,
        "n_results": results.len(),
        "results": results,
    }))
}

async fn ensembl_homology(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: HomologyArgs =
        serde_json::from_value(args.clone()).context("invalid Ensembl homology arguments")?;
    let gene_symbol = optional_ident(&args.gene_symbol, "gene_symbol", MAX_ID, &['-', '.'])?;
    let mut gene_id = optional_ident(&args.gene_id, "gene_id", MAX_ID, &['.', '-'])?;
    if gene_symbol.is_none() == gene_id.is_none() {
        bail!("pass exactly one of gene_symbol / gene_id");
    }
    let species = species_name(&args.species)?;
    let homology_type = enum_token(&args.homology_type, "homology_type", HOMOLOGY_TYPES)?;
    let target_species = optional_ident(&args.target_species, "target_species", MAX_ID, &['_'])?;
    if let Some(taxon) = args.target_taxon {
        if !(1..=99_999_999).contains(&taxon) {
            bail!("target_taxon must be a positive NCBI taxon id");
        }
    }
    let cap = require_cap(args.max_homologies, "max_homologies", MAX_HOMOLOGIES)?;
    let mut symbol = gene_symbol.clone();
    if gene_id.is_none() {
        let query = gene_symbol.as_deref().unwrap();
        let rec = ensembl_optional(
            bio,
            &format!(
                "/lookup/symbol/{}/{}",
                path_segment(&species),
                path_segment(query)
            ),
            &[],
        )
        .await?
        .with_context(|| {
            format!("no Ensembl gene found for symbol {query:?} in species {species:?}")
        })?;
        gene_id = Some(
            text(&rec, "id").with_context(|| format!("Ensembl lookup for {query:?} omitted id"))?,
        );
        if let Some(name) = text(&rec, "display_name") {
            symbol = Some(name);
        }
    }
    let gene_id = gene_id.unwrap();
    let mut params = vec![
        ("format".into(), "condensed".into()),
        ("type".into(), homology_type.clone()),
    ];
    if let Some(target) = &target_species {
        params.push(("target_species".into(), target.clone()));
    }
    if let Some(taxon) = args.target_taxon {
        params.push(("target_taxon".into(), taxon.to_string()));
    }
    let raw = ensembl_optional(
        bio,
        &format!(
            "/homology/id/{}/{}",
            path_segment(&species),
            path_segment(&gene_id)
        ),
        &params,
    )
    .await?;
    let mut rows = homology_rows(raw.as_ref())?;
    rows.sort_by(|a, b| {
        (
            text(a, "species").unwrap_or_default(),
            text(a, "id").unwrap_or_default(),
        )
            .cmp(&(
                text(b, "species").unwrap_or_default(),
                text(b, "id").unwrap_or_default(),
            ))
    });
    let total = rows.len();
    rows.truncate(cap);
    Ok(json!({
        "source": "Ensembl REST",
        "source_url": ENSEMBL_REST,
        "gene_id": gene_id,
        "gene_symbol": symbol,
        "species": species,
        "homology_type": homology_type,
        "target_species": target_species,
        "target_taxon": args.target_taxon,
        "n_total": total,
        "homologies_truncated": total > rows.len(),
        "homologies": rows,
    }))
}

async fn ensembl_sequence(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SequenceArgs =
        serde_json::from_value(args.clone()).context("invalid Ensembl sequence arguments")?;
    let stable_id = optional_ident(&args.stable_id, "stable_id", MAX_ID, &['.', '-'])?;
    let region = optional_region(&args.region)?;
    if stable_id.is_none() == region.is_none() {
        bail!("pass exactly one of stable_id / region");
    }
    let species = species_name(&args.species)?;
    let mut seq_type = enum_token(&args.seq_type, "seq_type", SEQ_TYPES)?;
    let max_bytes = require_cap(args.max_bytes, "max_bytes", MAX_BYTES)?;
    let (raw, query) = if let Some(id) = &stable_id {
        if seq_type == "protein" && !protein_seq_id(id) {
            bail!(
                "seq_type protein is only valid for transcript (ENST) or protein (ENSP) stable IDs"
            );
        }
        (
            ensembl_optional(
                bio,
                &format!("/sequence/id/{}", path_segment(id)),
                &[("type".into(), seq_type.clone())],
            )
            .await?,
            json!({"stable_id": id}),
        )
    } else {
        seq_type = "genomic".into();
        let region = region.unwrap();
        (
            Some(
                ensembl_json(
                    bio,
                    &format!(
                        "/sequence/region/{}/{}",
                        path_segment(&species),
                        path_segment(&region)
                    ),
                    &[],
                )
                .await?,
            ),
            json!({"region": region, "species": species}),
        )
    };
    let Some(raw) = raw else {
        return Ok(json!({
            "source": "Ensembl REST",
            "source_url": ENSEMBL_REST,
            "found": false,
            "query": query,
            "seq_type": seq_type,
            "id": Value::Null,
            "description": Value::Null,
            "molecule": Value::Null,
            "length": Value::Null,
            "sha256": Value::Null,
            "seq": Value::Null,
        }));
    };
    cap_sequence(raw, query, seq_type, max_bytes)
}

async fn ensembl_overlap_region(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: OverlapArgs =
        serde_json::from_value(args.clone()).context("invalid Ensembl overlap arguments")?;
    let region = region_token(&args.region)?;
    let feature = enum_token(&args.feature, "feature", OVERLAP_FEATURES)?;
    let species = species_name(&args.species)?;
    let cap = require_cap(args.max_features, "max_features", MAX_FEATURES)?;
    let raw = ensembl_json(
        bio,
        &format!(
            "/overlap/region/{}/{}",
            path_segment(&species),
            path_segment(&region)
        ),
        &[("feature".into(), feature.clone())],
    )
    .await?;
    let mut rows = match raw {
        Value::Array(rows) => rows,
        _ => bail!("Ensembl REST /overlap/region returned an unrecognized shape"),
    };
    rows.sort_by(|a, b| {
        (
            int_field(a, "start").unwrap_or(0),
            text(a, "id").unwrap_or_default(),
        )
            .cmp(&(
                int_field(b, "start").unwrap_or(0),
                text(b, "id").unwrap_or_default(),
            ))
    });
    let total = rows.len();
    rows.truncate(cap);
    Ok(json!({
        "source": "Ensembl REST",
        "source_url": ENSEMBL_REST,
        "region": region,
        "species": species,
        "feature": feature,
        "n_total": total,
        "features_truncated": total > rows.len(),
        "features": rows,
    }))
}

async fn ucsc_list_tracks(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ListTracksArgs =
        serde_json::from_value(args.clone()).context("invalid UCSC track listing arguments")?;
    let genome = genome_name(&args.genome)?;
    let filter = filter_token(args.filter_text.as_deref())?;
    let cap = require_cap(args.max_tracks, "max_tracks", MAX_TRACKS)?;
    let payload = ucsc_json(
        bio,
        "/list/tracks",
        &[
            ("genome".into(), genome.clone()),
            ("trackLeavesOnly".into(), "1".into()),
        ],
    )
    .await?;
    let mut rows = list_track_rows(&payload, &genome, filter.as_deref())?;
    rows.sort_by(|a, b| {
        text(a, "track")
            .unwrap_or_default()
            .cmp(&text(b, "track").unwrap_or_default())
    });
    let total = rows.len();
    rows.truncate(cap);
    Ok(json!({
        "source": "UCSC Genome Browser",
        "source_url": UCSC_API,
        "genome": genome,
        "filter_text": filter,
        "n_total": total,
        "tracks_truncated": total > rows.len(),
        "tracks": rows,
    }))
}

async fn ucsc_track_data(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: TrackDataArgs =
        serde_json::from_value(args.clone()).context("invalid UCSC track data arguments")?;
    let track = track_name(&args.track)?;
    let chrom = chrom_name(&args.chrom)?;
    let (start, end) = ucsc_interval(args.start, args.end)?;
    let genome = genome_name(&args.genome)?;
    let cap = require_cap(args.max_rows, "max_rows", MAX_ROWS)?;
    let payload = ucsc_track_payload(bio, &genome, &track, &chrom, start, end, Some(cap)).await?;
    let rows = extract_track_rows(&payload, &track, &chrom)?;
    let mut out = json!({
        "source": "UCSC Genome Browser",
        "source_url": UCSC_API,
        "genome": genome,
        "track": track,
        "chrom": chrom,
        "start": start,
        "end": end,
        "browser_url": ucsc_browser_url(&genome, &chrom, start, end),
        "track_type": payload.get("trackType"),
        "items_returned": payload.get("itemsReturned").cloned().unwrap_or_else(|| json!(rows.len())),
        "truncated": json_flag(payload.get("maxItemsLimit")),
        "rows": rows,
    });
    if let Some(url) = text(&payload, "dataDownloadUrl") {
        out["data_download_url"] = json!(url);
    }
    Ok(out)
}

async fn ucsc_conservation(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ConservationArgs =
        serde_json::from_value(args.clone()).context("invalid UCSC conservation arguments")?;
    let chrom = chrom_name(&args.chrom)?;
    let (start, end) = ucsc_interval(args.start, args.end)?;
    let span = end - start;
    if span > MAX_CONSERVATION_SPAN {
        bail!(
            "span {span} bp exceeds the {MAX_CONSERVATION_SPAN} bp cap — split the region into consecutive windows"
        );
    }
    let genome = genome_name(&args.genome)?;
    let track = track_name(&args.track)?;
    let cap = require_cap(args.max_values, "max_values", MAX_VALUES)?;
    let payload = ucsc_track_payload(bio, &genome, &track, &chrom, start, end, None).await?;
    if json_flag(payload.get("maxItemsLimit")) {
        bail!(
            "UCSC truncated the {track:?} listing for this span (itemsReturned={}) — the summary would be incomplete; query a smaller region",
            payload.get("itemsReturned").cloned().unwrap_or(Value::Null)
        );
    }
    let rows = extract_track_rows(&payload, &track, &chrom)?;
    let summary = summarize_conservation(&rows, start, end, payload.get("trackType"), &track)?;
    let mut out = json!({
        "source": "UCSC Genome Browser",
        "source_url": UCSC_API,
        "genome": genome,
        "track": track,
        "chrom": chrom,
        "start": start,
        "end": end,
        "browser_url": ucsc_browser_url(&genome, &chrom, start, end),
        "span_bp": span,
        "n_bases_covered": summary.covered,
        "coverage_fraction": summary.coverage_fraction,
        "mean": summary.mean,
        "min": summary.min,
        "max": summary.max,
    });
    if args.include_values {
        let total = summary.values.len();
        let mut values = summary.values;
        values.truncate(cap);
        out["values"] = json!(values);
        out["values_truncated"] = json!(total > cap);
        out["n_value_rows"] = json!(total);
    }
    Ok(out)
}

async fn ucsc_tfbs_clusters(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: TfbsArgs =
        serde_json::from_value(args.clone()).context("invalid UCSC TFBS arguments")?;
    let genome = genome_name(&args.genome)?;
    let track = tfbs_track(&genome)?;
    let chrom = chrom_name(&args.chrom)?;
    let (start, end) = ucsc_interval(args.start, args.end)?;
    let cap = require_cap(args.max_rows, "max_rows", MAX_ROWS)?;
    let payload = ucsc_track_payload(bio, &genome, track, &chrom, start, end, Some(cap)).await?;
    let rows = extract_track_rows(&payload, track, &chrom)?;
    let mut clusters: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "name": row.get("name"),
                "chrom": row.get("chrom"),
                "chromStart": row.get("chromStart"),
                "chromEnd": row.get("chromEnd"),
                "score": row.get("score"),
                "sourceCount": row.get("sourceCount"),
            })
        })
        .collect();
    clusters.sort_by(|a, b| {
        (
            int_field(a, "chromStart").unwrap_or(0),
            text(a, "name").unwrap_or_default(),
        )
            .cmp(&(
                int_field(b, "chromStart").unwrap_or(0),
                text(b, "name").unwrap_or_default(),
            ))
    });
    let mut factors: Vec<String> = clusters
        .iter()
        .filter_map(|row| text(row, "name"))
        .collect();
    factors.sort();
    factors.dedup();
    Ok(json!({
        "source": "UCSC Genome Browser",
        "source_url": UCSC_API,
        "genome": genome,
        "track": track,
        "chrom": chrom,
        "start": start,
        "end": end,
        "browser_url": ucsc_browser_url(&genome, &chrom, start, end),
        "items_returned": payload.get("itemsReturned").cloned().unwrap_or_else(|| json!(clusters.len())),
        "truncated": json_flag(payload.get("maxItemsLimit")),
        "n_factors": factors.len(),
        "factors": factors,
        "clusters": clusters,
    }))
}

async fn ucsc_chrom_sizes(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ChromSizesArgs = serde_json::from_value(args.clone())
        .context("invalid UCSC chromosome listing arguments")?;
    let genome = genome_name(&args.genome)?;
    let filter = filter_token(args.filter_text.as_deref())?;
    let cap = require_cap(args.max_chroms, "max_chroms", MAX_CHROMS)?;
    let payload = ucsc_json(
        bio,
        "/list/chromosomes",
        &[("genome".into(), genome.clone())],
    )
    .await?;
    let chroms = payload
        .get("chromosomes")
        .and_then(Value::as_object)
        .context("UCSC /list/chromosomes omitted the chromosomes map")?;
    let mut rows: Vec<Value> = chroms
        .iter()
        .filter_map(|(name, size)| {
            as_size(size).map(|size_bp| json!({"name": name, "size_bp": size_bp}))
        })
        .collect();
    if let Some(needle) = &filter {
        let needle = needle.to_ascii_lowercase();
        rows.retain(|row| {
            text(row, "name")
                .map(|name| name.to_ascii_lowercase().contains(&needle))
                .unwrap_or(false)
        });
    }
    rows.sort_by(|a, b| {
        let sa = a.get("size_bp").and_then(Value::as_u64).unwrap_or(0);
        let sb = b.get("size_bp").and_then(Value::as_u64).unwrap_or(0);
        sb.cmp(&sa).then_with(|| {
            text(a, "name")
                .unwrap_or_default()
                .cmp(&text(b, "name").unwrap_or_default())
        })
    });
    let total = rows.len();
    rows.truncate(cap);
    Ok(json!({
        "source": "UCSC Genome Browser",
        "source_url": UCSC_API,
        "genome": genome,
        "filter_text": filter,
        "chrom_count": payload.get("chromCount"),
        "n_total": total,
        "chroms_truncated": total > rows.len(),
        "chromosomes": rows,
    }))
}

async fn ensembl_json(bio: &NativeBio, path: &str, params: &[(String, String)]) -> Result<Value> {
    let response = bio
        .http()
        .send(
            ENSEMBL,
            Method::GET,
            &format!("{}{path}", ensembl_base(bio)),
            params,
        )
        .await?;
    response.check()?;
    serde_json::from_slice(&response.body).context("Ensembl REST returned invalid JSON")
}

async fn ensembl_optional(
    bio: &NativeBio,
    path: &str,
    params: &[(String, String)],
) -> Result<Option<Value>> {
    let response = bio
        .http()
        .send(
            ENSEMBL,
            Method::GET,
            &format!("{}{path}", ensembl_base(bio)),
            params,
        )
        .await?;
    if is_not_found(response.status.as_u16()) {
        return Ok(None);
    }
    response.check()?;
    Ok(Some(
        serde_json::from_slice(&response.body).context("Ensembl REST returned invalid JSON")?,
    ))
}

async fn ucsc_json(bio: &NativeBio, path: &str, params: &[(String, String)]) -> Result<Value> {
    let response = bio
        .http()
        .send(
            UCSC,
            Method::GET,
            &format!("{}{path}", ucsc_base(bio)),
            params,
        )
        .await?;
    response.check()?;
    serde_json::from_slice(&response.body).context("UCSC Genome Browser returned invalid JSON")
}

async fn ucsc_track_payload(
    bio: &NativeBio,
    genome: &str,
    track: &str,
    chrom: &str,
    start: i64,
    end: i64,
    max_items: Option<usize>,
) -> Result<Value> {
    let mut params = vec![
        ("genome".into(), genome.to_string()),
        ("track".into(), track.to_string()),
        ("chrom".into(), chrom.to_string()),
        ("start".into(), start.to_string()),
        ("end".into(), end.to_string()),
    ];
    if let Some(max) = max_items {
        params.push(("maxItemsOutput".into(), max.to_string()));
    }
    ucsc_json(bio, "/getData/track", &params).await
}

fn ensembl_base(bio: &NativeBio) -> String {
    bio.credential("ENSEMBL_BASE_URL")
        .map(|value| value.trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| ENSEMBL_REST.to_string())
}

fn ucsc_base(bio: &NativeBio) -> String {
    bio.credential("UCSC_BASE_URL")
        .map(|value| value.trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| UCSC_API.to_string())
}

fn is_not_found(status: u16) -> bool {
    status == 400 || status == 404
}

fn looks_like_stable_id(query: &str) -> bool {
    let upper = query.trim().to_ascii_uppercase();
    let core = match upper.split_once('.') {
        Some((head, version))
            if !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit()) =>
        {
            head
        }
        _ => upper.as_str(),
    };
    if let Some(rest) = core.strip_prefix("LRG_") {
        return !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit());
    }
    let Some(rest) = core.strip_prefix("ENS") else {
        return false;
    };
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_uppercase() {
        i += 1;
    }
    if i == 0 {
        return false;
    }
    let feature = bytes[i - 1];
    if !matches!(feature, b'G' | b'T' | b'P' | b'E' | b'R') {
        return false;
    }
    let digits = &bytes[i..];
    digits.len() >= 6 && digits.iter().all(|b| b.is_ascii_digit())
}

fn protein_seq_id(id: &str) -> bool {
    let upper = id.trim().to_ascii_uppercase();
    let core = match upper.split_once('.') {
        Some((head, version))
            if !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit()) =>
        {
            head
        }
        _ => upper.as_str(),
    };
    let Some(rest) = core.strip_prefix("ENS") else {
        return false;
    };
    rest.contains('T') || rest.contains('P')
}

fn ident(value: &str, name: &str, max: usize, extra: &[char]) -> Result<String> {
    let token = value.trim();
    if token.is_empty() || token.len() > max {
        bail!("{name} must be 1 to {max} characters");
    }
    if token
        .chars()
        .any(|c| !(c.is_ascii_alphanumeric() || c == '_' || extra.contains(&c)))
    {
        bail!("{name} {token:?} is not a valid identifier");
    }
    Ok(token.to_string())
}

fn optional_ident(
    value: &Option<String>,
    name: &str,
    max: usize,
    extra: &[char],
) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(raw) if raw.trim().is_empty() => Ok(None),
        Some(raw) => Ok(Some(ident(raw, name, max, extra)?)),
    }
}

fn species_name(value: &str) -> Result<String> {
    ident(value, "species", MAX_ID, &['_'])
}

fn genome_name(value: &str) -> Result<String> {
    ident(value, "genome", 32, &[])
}

fn chrom_name(value: &str) -> Result<String> {
    ident(value, "chrom", 64, &['_'])
}

fn track_name(value: &str) -> Result<String> {
    ident(value, "track", 128, &[])
}

fn region_token(value: &str) -> Result<String> {
    let token = value.trim();
    if token.len() < 3 || token.len() > MAX_REGION {
        bail!("region must be 3 to {MAX_REGION} characters (chrom:start-end)");
    }
    if token
        .chars()
        .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_' | '.')))
    {
        bail!("region {token:?} is not a valid Ensembl region (chrom:start-end)");
    }
    if !token.contains(':') {
        bail!("region must be chrom:start-end (1-based inclusive)");
    }
    Ok(token.to_string())
}

fn optional_region(value: &Option<String>) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(raw) if raw.trim().is_empty() => Ok(None),
        Some(raw) => Ok(Some(region_token(raw)?)),
    }
}

fn allele_token(value: &str) -> Result<String> {
    let token = value.trim();
    if token.is_empty() || token.len() > MAX_ALLELE {
        bail!("allele must be 1 to {MAX_ALLELE} characters");
    }
    if !token
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'*')
    {
        bail!("allele must be a nucleotide string or '-' for a deletion");
    }
    Ok(token.to_string())
}

fn optional_allele(value: &Option<String>) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(raw) if raw.trim().is_empty() => Ok(None),
        Some(raw) => Ok(Some(allele_token(raw)?)),
    }
}

fn filter_token(value: Option<&str>) -> Result<Option<String>> {
    let Some(token) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if token.len() > MAX_FILTER {
        bail!("filter_text exceeds {MAX_FILTER} characters");
    }
    if token
        .chars()
        .any(|c| c.is_control() || c == '/' || c == '\\')
    {
        bail!("filter_text contains unsupported characters");
    }
    Ok(Some(token.to_string()))
}

fn enum_token(value: &str, name: &str, allowed: &[&str]) -> Result<String> {
    let token = value.trim();
    if allowed.iter().any(|item| *item == token) {
        Ok(token.to_string())
    } else {
        bail!("{name} must be one of {}", allowed.join(", "))
    }
}

fn require_cap(n: u32, name: &str, max: u32) -> Result<usize> {
    if n < 1 || n > max {
        bail!("{name} must be between 1 and {max}");
    }
    Ok(n as usize)
}

fn ucsc_interval(start: i64, end: i64) -> Result<(i64, i64)> {
    if start < 0 || end < 0 || start > MAX_COORD || end > MAX_COORD {
        bail!("UCSC coordinates must be between 0 and {MAX_COORD} (0-based half-open)");
    }
    if end <= start {
        bail!("end must be greater than start (0-based half-open)");
    }
    Ok((start, end))
}

fn tfbs_track(genome: &str) -> Result<&'static str> {
    match genome {
        "hg38" => Ok("encRegTfbsClustered"),
        "hg19" => Ok("wgEncodeRegTfbsClusteredV3"),
        _ => bail!(
            "no ENCODE TFBS cluster track known for genome {genome:?} — supported: hg19, hg38"
        ),
    }
}

fn path_segment(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn text(value: &Value, key: &str) -> Option<String> {
    match value.get(key) {
        Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
        Some(Value::Number(number)) => Some(number.to_string()),
        _ => None,
    }
}

fn int_field(value: &Value, key: &str) -> Option<i64> {
    match value.get(key) {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_u64().map(|n| n as i64)),
        Some(Value::String(text)) => text.parse().ok(),
        _ => None,
    }
}

fn as_size(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_i64().and_then(|n| u64::try_from(n).ok())),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn json_flag(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(true)) => true,
        Some(Value::Number(number)) => number.as_u64() == Some(1) || number.as_i64() == Some(1),
        Some(Value::String(text)) => text.eq_ignore_ascii_case("true") || text == "1",
        _ => false,
    }
}

fn impact_rank(impact: Option<&str>) -> u8 {
    impact
        .and_then(|name| {
            IMPACT_RANK
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, rank)| *rank)
        })
        .unwrap_or(9)
}

fn ensembl_browser_url(species: &str, record: &Value) -> Option<String> {
    let id = text(record, "id")?;
    let species = text(record, "species").unwrap_or_else(|| species.to_string());
    let object = text(record, "object_type").unwrap_or_else(|| "Gene".into());
    let path = match object.as_str() {
        "Transcript" => format!("Transcript/Summary?t={id}"),
        "Translation" => format!("Transcript/ProteinSummary?p={id}"),
        _ => format!("Gene/Summary?g={id}"),
    };
    Some(format!("{ENSEMBL_BROWSER}/{species}/{path}"))
}

fn ucsc_browser_url(genome: &str, chrom: &str, start: i64, end: i64) -> String {
    format!(
        "{UCSC_BROWSER}?db={genome}&position={chrom}:{}-{end}",
        start + 1
    )
}

fn vep_results(raw: &Value, cap: usize) -> Result<Vec<Value>> {
    let items = match raw {
        Value::Array(rows) => rows.clone(),
        Value::Object(map)
            if map.contains_key("most_severe_consequence") || map.contains_key("input") =>
        {
            vec![raw.clone()]
        }
        Value::Object(map) if map.contains_key("error") => {
            bail!("Ensembl VEP rejected the request")
        }
        _ => bail!("Ensembl REST /vep returned an unrecognized shape"),
    };
    Ok(items.iter().map(|item| summarize_vep(item, cap)).collect())
}

fn summarize_vep(raw: &Value, cap: usize) -> Value {
    let mut tx: Vec<Value> = raw
        .get("transcript_consequences")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    tx.sort_by(|a, b| {
        (
            impact_rank(a.get("impact").and_then(Value::as_str)),
            text(a, "gene_id").unwrap_or_default(),
            text(a, "transcript_id").unwrap_or_default(),
        )
            .cmp(&(
                impact_rank(b.get("impact").and_then(Value::as_str)),
                text(b, "gene_id").unwrap_or_default(),
                text(b, "transcript_id").unwrap_or_default(),
            ))
    });
    let mut genes: Map<String, Value> = Map::new();
    for row in &tx {
        let gid = text(row, "gene_id").unwrap_or_else(|| "?".into());
        let entry = genes.entry(gid.clone()).or_insert_with(|| {
            json!({
                "gene_id": row.get("gene_id"),
                "gene_symbol": row.get("gene_symbol"),
                "worst_impact": row.get("impact"),
                "n_transcripts": 0,
            })
        });
        if let Some(count) = entry.get("n_transcripts").and_then(Value::as_u64) {
            entry["n_transcripts"] = json!(count + 1);
        }
        let current = entry.get("worst_impact").and_then(Value::as_str);
        let incoming = row.get("impact").and_then(Value::as_str);
        if impact_rank(incoming) < impact_rank(current) {
            entry["worst_impact"] = json!(incoming);
        }
    }
    let mut gene_rows: Vec<Value> = genes.into_values().collect();
    gene_rows.sort_by(|a, b| {
        (
            impact_rank(a.get("worst_impact").and_then(Value::as_str)),
            text(a, "gene_id").unwrap_or_default(),
        )
            .cmp(&(
                impact_rank(b.get("worst_impact").and_then(Value::as_str)),
                text(b, "gene_id").unwrap_or_default(),
            ))
    });
    let total = tx.len();
    tx.truncate(cap);
    let colocated = raw
        .get("colocated_variants")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    let mut out = Map::new();
                    for key in [
                        "id",
                        "allele_string",
                        "clin_sig",
                        "clin_sig_allele",
                        "somatic",
                        "phenotype_or_disease",
                        "start",
                        "end",
                    ] {
                        if let Some(value) = row.get(key) {
                            if !value.is_null() {
                                out.insert(key.to_string(), value.clone());
                            }
                        }
                    }
                    Value::Object(out)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "input": raw.get("input"),
        "assembly_name": raw.get("assembly_name"),
        "seq_region_name": raw.get("seq_region_name"),
        "start": raw.get("start"),
        "end": raw.get("end"),
        "strand": raw.get("strand"),
        "allele_string": raw.get("allele_string"),
        "most_severe_consequence": raw.get("most_severe_consequence"),
        "genes": gene_rows,
        "n_transcript_consequences": total,
        "transcript_consequences_truncated": total > tx.len(),
        "transcript_consequences": tx,
        "n_regulatory_feature_consequences": raw.get("regulatory_feature_consequences").and_then(Value::as_array).map(|rows| rows.len()).unwrap_or(0),
        "n_motif_feature_consequences": raw.get("motif_feature_consequences").and_then(Value::as_array).map(|rows| rows.len()).unwrap_or(0),
        "colocated_variants": colocated,
    })
}

fn homology_rows(raw: Option<&Value>) -> Result<Vec<Value>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let data = raw
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let Some(first) = data.first() else {
        return Ok(Vec::new());
    };
    let rows = first
        .get("homologies")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(rows.iter().map(homology_row).collect())
}

fn homology_row(raw: &Value) -> Value {
    if raw.get("id").is_some() && raw.get("species").is_some() {
        return raw.clone();
    }
    let target = raw.get("target").unwrap_or(raw);
    json!({
        "type": raw.get("type"),
        "species": target.get("species"),
        "id": target.get("id"),
        "protein_id": target.get("protein_id"),
        "taxonomy_level": raw.get("taxonomy_level"),
        "method_link_type": raw.get("method_link_type"),
    })
}

fn cap_sequence(raw: Value, query: Value, seq_type: String, max_bytes: usize) -> Result<Value> {
    let seq = text(&raw, "seq").unwrap_or_default();
    let bytes = seq.len();
    let mut result = json!({
        "source": "Ensembl REST",
        "source_url": ENSEMBL_REST,
        "found": true,
        "query": query,
        "seq_type": seq_type,
        "id": raw.get("id"),
        "description": raw.get("desc"),
        "molecule": raw.get("molecule"),
        "length": bytes,
        "sha256": sha256_hex(seq.as_bytes()),
        "seq": seq,
    });
    if bytes > max_bytes {
        if let Some(map) = result.as_object_mut() {
            map.remove("seq");
            map.insert(
                "seq_omitted".into(),
                json!(format!(
                    "seq is {bytes} bytes > max_bytes={max_bytes}; metadata, length and sha256 are included — re-call with a larger max_bytes to get the full text"
                )),
            );
        }
    }
    Ok(result)
}

fn list_track_rows(payload: &Value, genome: &str, filter: Option<&str>) -> Result<Vec<Value>> {
    let obj = payload
        .as_object()
        .context("UCSC /list/tracks returned an unrecognized shape")?;
    let tracks = match obj.get(genome) {
        Some(Value::Object(map)) => map,
        _ if obj.values().any(Value::is_object) && !obj.contains_key("error") => obj,
        _ => {
            bail!("UCSC Genome Browser /list/tracks did not include a track listing for genome {genome:?}")
        }
    };
    let needle = filter.map(|value| value.to_ascii_lowercase());
    let mut rows = Vec::new();
    for (name, meta) in tracks {
        let Some(meta) = meta.as_object() else {
            continue;
        };
        if let Some(needle) = &needle {
            let hay = format!(
                "{} {} {}",
                name,
                meta.get("shortLabel").and_then(Value::as_str).unwrap_or(""),
                meta.get("longLabel").and_then(Value::as_str).unwrap_or("")
            )
            .to_ascii_lowercase();
            if !hay.contains(needle) {
                continue;
            }
        }
        rows.push(json!({
            "track": name,
            "short_label": meta.get("shortLabel"),
            "long_label": meta.get("longLabel"),
            "type": meta.get("type"),
            "group": meta.get("group"),
            "parent": meta.get("parent"),
        }));
    }
    Ok(rows)
}

fn extract_track_rows(payload: &Value, track: &str, chrom: &str) -> Result<Vec<Value>> {
    let items = payload
        .get("itemsReturned")
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_i64().map(|n| n.max(0) as u64))
        })
        .unwrap_or(0);
    match payload.get(track) {
        Some(Value::Array(rows)) => Ok(rows.clone()),
        Some(Value::Object(map)) => Ok(map
            .get(chrom)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()),
        _ => {
            let lists: Vec<&Vec<Value>> = payload
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(key, value)| value.is_array() && !is_track_meta_key(key))
                .filter_map(|(_, value)| value.as_array())
                .collect();
            if lists.len() == 1 {
                Ok(lists[0].clone())
            } else if items > 0 {
                bail!(
                    "unrecognised UCSC payload shape for track {track:?}: itemsReturned={items} but no single row list under the track name"
                );
            } else {
                Ok(Vec::new())
            }
        }
    }
}

fn is_track_meta_key(key: &str) -> bool {
    matches!(
        key,
        "downloadTime"
            | "downloadTimeStamp"
            | "dataTime"
            | "dataTimeStamp"
            | "genome"
            | "trackType"
            | "track"
            | "chrom"
            | "start"
            | "end"
            | "itemsReturned"
            | "maxItemsLimit"
            | "dataDownloadUrl"
            | "hubUrl"
    )
}

struct ConservationSummary {
    covered: i64,
    coverage_fraction: f64,
    mean: Option<f64>,
    min: Option<f64>,
    max: Option<f64>,
    values: Vec<Value>,
}

fn summarize_conservation(
    rows: &[Value],
    start: i64,
    end: i64,
    track_type: Option<&Value>,
    track: &str,
) -> Result<ConservationSummary> {
    let span = end - start;
    let mut covered = 0i64;
    let mut total = 0.0;
    let mut vmin: Option<f64> = None;
    let mut vmax: Option<f64> = None;
    let mut values = Vec::new();
    for row in rows {
        if row.get("value").is_none() {
            bail!(
                "track {track:?} (type {}) returns rows without per-base values — ucsc_conservation needs a wiggle/bigWig score track (phyloP*/phastCons*); use ucsc_track_data for BED-like tracks",
                track_type.cloned().unwrap_or(Value::Null)
            );
        }
        let rs = int_field(row, "start").unwrap_or(0).max(start);
        let re = int_field(row, "end").unwrap_or(0).min(end);
        let width = re - rs;
        if width <= 0 {
            continue;
        }
        let value = match row.get("value") {
            Some(Value::Number(number)) => number
                .as_f64()
                .context("conservation value is not finite")?,
            Some(Value::String(text)) => text
                .parse::<f64>()
                .context("conservation value is not a number")?,
            _ => bail!("conservation value is missing"),
        };
        if !value.is_finite() {
            bail!("conservation value is not finite");
        }
        covered += width;
        total += value * width as f64;
        vmin = Some(vmin.map_or(value, |current| current.min(value)));
        vmax = Some(vmax.map_or(value, |current| current.max(value)));
        values.push(json!({"start": rs, "end": re, "value": value}));
    }
    Ok(ConservationSummary {
        covered,
        coverage_fraction: if span > 0 {
            round6(covered as f64 / span as f64)
        } else {
            0.0
        },
        mean: if covered > 0 {
            Some(round6(total / covered as f64))
        } else {
            None
        },
        min: vmin,
        max: vmax,
        values,
    })
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut hash = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).saturating_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().enumerate().take(16) {
            *word = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut a = hash[0];
        let mut b = hash[1];
        let mut c = hash[2];
        let mut d = hash[3];
        let mut e = hash[4];
        let mut f = hash[5];
        let mut g = hash[6];
        let mut h = hash[7];
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        hash[0] = hash[0].wrapping_add(a);
        hash[1] = hash[1].wrapping_add(b);
        hash[2] = hash[2].wrapping_add(c);
        hash[3] = hash[3].wrapping_add(d);
        hash[4] = hash[4].wrapping_add(e);
        hash[5] = hash[5].wrapping_add(f);
        hash[6] = hash[6].wrapping_add(g);
        hash[7] = hash[7].wrapping_add(h);
    }
    hash.iter().map(|word| format!("{word:08x}")).collect()
}
