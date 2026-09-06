//! Official eQTL metadata and tabix-indexed summary statistics, fetched in bounded
//! HTTP ranges. No local download of a whole study or external tabix process.
use crate::{
    http::{Source, MAX_RESPONSE},
    NativeBio,
};
use anyhow::{bail, Context, Result};
use noodles_core::{region::Interval, Position};
use noodles_csi::BinningIndex;
use reqwest::Method;
use serde_json::{json, Value};
use std::{
    io::{BufRead, Cursor},
    time::Duration,
};

pub(super) const METADATA: &str = "https://api.github.com/repos/eQTL-Catalogue/eQTL-Catalogue-resources/contents/tabix/tabix_ftp_paths.tsv";
static METADATA_CACHE: tokio::sync::OnceCell<Vec<Value>> = tokio::sync::OnceCell::const_new();
const FTP_ROOT: &str = "ftp://ftp.ebi.ac.uk/pub/databases/spot/eQTL/sumstats/";
const HTTPS_ROOT: &str = "https://ftp.ebi.ac.uk/pub/databases/spot/eQTL/sumstats/";
const SOURCE: Source = Source("eQTL summary files", Duration::from_secs(2));
const LOOKUP: Source = Source("Ensembl REST", Duration::from_millis(350));
const MAX_SCAN: usize = 16 * 1024 * 1024;
const BLOCK: u64 = 65536;

pub(super) async fn metadata(bio: &NativeBio) -> Result<Vec<Value>> {
    if let Some(url) = bio.credential("EQTL_METADATA_URL") {
        return fetch_metadata(bio, url).await;
    }
    // One public metadata snapshot per process avoids spending anonymous GitHub
    // quota for every locus; failed fetches are not cached.
    Ok(METADATA_CACHE
        .get_or_try_init(|| fetch_metadata(bio, METADATA))
        .await?
        .clone())
}

async fn fetch_metadata(bio: &NativeBio, url: &str) -> Result<Vec<Value>> {
    let text = bio
        .http()
        .get_accept(SOURCE, url, "application/vnd.github.raw+json")
        .await?
        .text()?;
    parse_metadata(&text)
}

fn parse_metadata(text: &str) -> Result<Vec<Value>> {
    let mut lines = text.lines();
    let header: Vec<_> = lines
        .next()
        .context("eQTL metadata is empty")?
        .split('\t')
        .collect();
    for field in [
        "dataset_id",
        "study_id",
        "study_label",
        "tissue_label",
        "quant_method",
        "ftp_path",
    ] {
        if !header.contains(&field) {
            bail!("eQTL metadata omitted {field}");
        }
    }
    let mut rows = Vec::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let values: Vec<_> = line.split('\t').collect();
        if values.len() != header.len() {
            bail!("eQTL metadata has inconsistent columns");
        }
        let mut row = serde_json::Map::new();
        for (key, value) in header.iter().zip(values) {
            row.insert(
                (*key).into(),
                if *key == "sample_size" {
                    json!(value.parse::<u64>().ok())
                } else {
                    json!(value)
                },
            );
        }
        let public_url = public_data_url(row["ftp_path"].as_str().unwrap_or(""))?;
        row.insert("source_url".into(), json!(public_url));
        rows.push(Value::Object(row));
    }
    if rows.is_empty() {
        bail!("eQTL metadata contains no datasets");
    }
    Ok(rows)
}

fn public_data_url(path: &str) -> Result<String> {
    let relative = path
        .strip_prefix(FTP_ROOT)
        .context("eQTL metadata contains an unexpected file host")?;
    if relative.contains("..")
        || relative.contains(['?', '#', '\\'])
        || !relative.ends_with(".tsv.gz")
    {
        bail!("eQTL metadata contains an invalid summary path");
    }
    Ok(format!("{HTTPS_ROOT}{relative}"))
}

fn data_url(bio: &NativeBio, public: &str) -> String {
    match bio.credential("EQTL_FILES_URL") {
        Some(base) => format!(
            "{}/{}",
            base.trim_end_matches('/'),
            public.strip_prefix(HTTPS_ROOT).unwrap()
        ),
        None => public.into(),
    }
}

#[derive(Debug)]
pub(super) struct Region {
    pub chrom: String,
    pub start: usize,
    pub end: usize,
}

impl Region {
    fn new(chrom: &str, start: usize, end: usize) -> Result<Self> {
        if !matches!(chrom, "X" | "Y" | "MT" | "M")
            && !chrom.parse::<u8>().is_ok_and(|n| (1..=22).contains(&n))
        {
            bail!("eQTL region must use a primary GRCh38 chromosome");
        }
        if start == 0 || end < start || end - start > 2_000_000 {
            bail!("eQTL region must be 1-based and span at most 2,000,001 bases");
        }
        Ok(Self {
            chrom: chrom.into(),
            start,
            end,
        })
    }
    pub fn label(&self) -> String {
        format!("{}:{}-{}", self.chrom, self.start, self.end)
    }
}

pub(super) async fn resolve_region(
    bio: &NativeBio,
    filters: &[(String, String)],
) -> Result<Region> {
    let arg = |key: &str| {
        filters
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    if let Some(pos) = arg("pos") {
        let (chrom, span) = pos.split_once(':').context("invalid eQTL region")?;
        let (start, end) = span.split_once('-').context("invalid eQTL region")?;
        return Region::new(chrom, start.parse()?, end.parse()?);
    }
    if let Some(variant) = arg("variant") {
        let parts: Vec<_> = variant.split('_').collect();
        let pos: usize = parts[1].parse()?;
        return Region::new(parts[0].trim_start_matches("chr"), pos, pos);
    }
    let base = bio
        .credential("EQTL_ENSEMBL_URL")
        .unwrap_or("https://rest.ensembl.org");
    if let Some(rsid) = arg("rsid") {
        let raw = bio
            .http()
            .send(
                LOOKUP,
                Method::GET,
                &format!("{base}/variation/human/{rsid}"),
                &[("content-type".into(), "application/json".into())],
            )
            .await?
            .json()?;
        let mappings: Vec<_> = raw["mappings"]
            .as_array()
            .context("Ensembl omitted variant mappings")?
            .iter()
            .filter(|r| r["assembly_name"] == "GRCh38" && r["location"].as_str().is_some())
            .collect();
        let row = mappings
            .first()
            .context("rsID has no GRCh38 mapping; supply an explicit pos region")?;
        if mappings.len() != 1 {
            bail!("rsID has multiple GRCh38 mappings; supply an explicit pos region");
        }
        let pos = row["start"]
            .as_u64()
            .context("Ensembl omitted variant position")? as usize;
        return Region::new(
            row["seq_region_name"]
                .as_str()
                .context("Ensembl omitted chromosome")?,
            pos,
            pos,
        );
    }
    let gene = arg("gene_id").context("eQTL query needs a gene, variant, rsID or region")?;
    let raw = bio
        .http()
        .send(
            LOOKUP,
            Method::GET,
            &format!("{base}/lookup/id/{gene}"),
            &[("content-type".into(), "application/json".into())],
        )
        .await?
        .json()?;
    let tss = raw[if raw["strand"] == -1 { "end" } else { "start" }]
        .as_u64()
        .context("Ensembl omitted gene coordinates")? as usize;
    Region::new(
        raw["seq_region_name"]
            .as_str()
            .context("Ensembl omitted chromosome")?,
        tss.saturating_sub(1_000_000).max(1),
        tss + 1_000_000,
    )
}

pub(super) async fn query(
    bio: &NativeBio,
    dataset: &Value,
    region: &Region,
    filters: &[(String, String)],
    cap: usize,
) -> Result<(Vec<Value>, bool)> {
    let url = data_url(
        bio,
        dataset["source_url"]
            .as_str()
            .context("eQTL dataset omitted source URL")?,
    );
    let response = bio
        .http()
        .ebi_download(SOURCE, &format!("{url}.tbi"))
        .await?;
    response.check()?;
    let index_bytes = response.body;
    let index = noodles_tabix::io::Reader::new(Cursor::new(index_bytes))
        .read_index()
        .context("invalid eQTL tabix index")?;
    let names = index
        .header()
        .context("eQTL index omitted chromosome names")?
        .reference_sequence_names();
    let Some(id) = names.get_index_of(region.chrom.as_bytes()) else {
        return Ok((Vec::new(), false));
    };
    let chunks = index.query(
        id,
        Interval::from(Position::try_from(region.start)?..=Position::try_from(region.end)?),
    )?;
    if chunks.is_empty() {
        return Ok((Vec::new(), false));
    }
    let first = bio.http().range(SOURCE, &url, 0, BLOCK - 1).await?;
    let mut first = noodles_bgzf::io::Reader::new(Cursor::new(first));
    let mut header = String::new();
    first
        .read_line(&mut header)
        .context("eQTL summary omitted its header")?;
    let header: Vec<_> = header
        .trim_end()
        .trim_start_matches('#')
        .split('\t')
        .map(str::to_string)
        .collect();
    for key in ["chromosome", "position", "variant", "pvalue", "gene_id"] {
        if !header.iter().any(|s| s == key) {
            bail!("eQTL summary omitted {key}");
        }
    }
    let mut rows = Vec::new();
    let mut scanned = 0usize;
    let mut downloaded = 0usize;
    for chunk in chunks {
        let start = chunk.start().compressed();
        let requested_end = chunk
            .end()
            .compressed()
            .checked_add(BLOCK - 1)
            .context("invalid eQTL chunk")?;
        let end = requested_end.min(start + MAX_RESPONSE as u64 - 1);
        if downloaded + (end - start + 1) as usize > 8 * 1024 * 1024 {
            return Ok((rows, true));
        }
        let bytes = bio.http().range(SOURCE, &url, start, end).await?;
        downloaded += bytes.len();
        let mut reader = noodles_bgzf::io::Reader::new(Cursor::new(bytes));
        reader.seek(
            noodles_bgzf::VirtualPosition::new(0, chunk.start().uncompressed())
                .context("invalid eQTL offset")?,
        )?;
        let stop = noodles_bgzf::VirtualPosition::new(
            chunk.end().compressed() - start,
            chunk.end().uncompressed(),
        )
        .context("invalid eQTL offset")?;
        let mut line = String::new();
        while reader.virtual_position() < stop {
            line.clear();
            let n = match reader.read_line(&mut line) {
                Ok(n) => n,
                Err(error)
                    if end < requested_end && error.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok((rows, true))
                }
                Err(error) => {
                    return Err(error).context("eQTL summary block is incomplete or corrupt")
                }
            };
            if n == 0 {
                bail!("eQTL summary ended before its indexed chunk");
            }
            scanned += n;
            if scanned > MAX_SCAN {
                return Ok((rows, true));
            }
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let row = parse_row(&header, &line)?;
            if matches_row(&row, region, filters) {
                if rows.len() == cap {
                    return Ok((rows, true));
                }
                rows.push(row);
            }
        }
    }
    Ok((rows, false))
}

fn parse_row(header: &[String], line: &str) -> Result<Value> {
    let cols: Vec<_> = line.trim_end_matches(['\r', '\n']).split('\t').collect();
    if cols.len() != header.len() {
        bail!("eQTL summary has inconsistent columns");
    }
    let mut row = serde_json::Map::new();
    for (key, value) in header.iter().zip(cols) {
        let value = if matches!(value, "NA" | "NaN" | "") {
            Value::Null
        } else if matches!(key.as_str(), "position" | "ac" | "an") {
            json!(value
                .parse::<u64>()
                .context("eQTL summary contains an invalid count or position")?)
        } else if matches!(
            key.as_str(),
            "beta" | "se" | "pvalue" | "maf" | "r2" | "median_tpm"
        ) {
            let n: f64 = value
                .parse()
                .context("eQTL summary contains an invalid numeric field")?;
            if !n.is_finite() {
                bail!("eQTL summary contains a nonfinite numeric field");
            }
            json!(n)
        } else {
            json!(value)
        };
        row.insert(key.clone(), value);
    }
    let p = row.get("pvalue").and_then(Value::as_f64);
    if let Some(p) = p {
        if !(0.0..=1.0).contains(&p) {
            bail!("eQTL summary contains an invalid p-value");
        }
        row.insert(
            "nlog10p".into(),
            json!(if p > 0.0 { Some(-p.log10()) } else { None }),
        );
    }
    Ok(Value::Object(row))
}

fn matches_row(row: &Value, region: &Region, filters: &[(String, String)]) -> bool {
    let pos = row["position"].as_f64().unwrap_or(0.0);
    row["chromosome"] == region.chrom
        && pos >= region.start as f64
        && pos <= region.end as f64
        && filters.iter().all(|(key, value)| match key.as_str() {
            "pos" => true,
            "nlog10p" => row["pvalue"].as_f64().is_some_and(|p| {
                p <= 10.0_f64.powf(-value.parse::<f64>().unwrap_or(f64::INFINITY))
            }),
            _ => row
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|s| s.split(';').any(|s| s == value)),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_paths_and_regions_are_bounded() {
        assert!(parse_metadata("wrong\theader\nvalue\tvalue").is_err());
        assert!(public_data_url("ftp://example.test/data.tsv.gz").is_err());
        assert!(public_data_url(&format!("{FTP_ROOT}../data.tsv.gz")).is_err());
        assert!(Region::new("19", 0, 2).is_err());
        assert!(Region::new("19", 2, 1).is_err());
        assert!(Region::new("19", 1, 2_000_002).is_err());
        assert!(Region::new("19_alt", 1, 2).is_err());
    }

    #[test]
    fn summary_rows_keep_integer_positions_and_zero_pvalues() {
        let header = ["chromosome", "position", "pvalue", "gene_id", "rsid"].map(str::to_string);
        let row = parse_row(&header, "19\t12\t0\tENSG00000000001\trs1\r\n").unwrap();
        assert_eq!(row["position"].as_u64(), Some(12));
        assert!(row["nlog10p"].is_null()); // Infinite -log10(0) has no finite JSON representation.
        let region = Region::new("19", 10, 20).unwrap();
        assert!(matches_row(
            &row,
            &region,
            &[("nlog10p".into(), "5".into())]
        ));
        assert!(!matches_row(
            &row,
            &region,
            &[("rsid".into(), "rs2".into())]
        ));
        assert!(parse_row(&header, "19\t12\t-0.1\tx\trs1").is_err());
        assert!(parse_row(&header, "19\twrong\t0.1\tx\trs1").is_err());
    }
}
