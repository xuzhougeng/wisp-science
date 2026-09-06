use super::{
    dbsnp_url, json_i64, json_string, ncbi_json, normalize_chrom, page_bound, require_region,
    require_rsid, variation_base, NativeBio, BATCH_DEADLINE, DBSNP_BROWSER,
};
use crate::http::NCBI;
use anyhow::{bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashSet};
use std::time::Instant;

const MAX_CITATIONS: usize = 20;
const MAX_RSIDS: usize = 20;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GetRsids {
    rsids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Region {
    chrom: String,
    start: i64,
    stop: i64,
    #[serde(default = "default_assembly")]
    assembly: String,
    #[serde(default = "default_max")]
    max_rsids: u32,
}

fn default_assembly() -> String {
    "GRCh38".into()
}

fn default_max() -> u32 {
    200
}

pub(super) async fn get_rsids(bio: &NativeBio, args: &Value) -> Result<Value> {
    let _ = super::contact_email(bio)?;
    let args: GetRsids =
        serde_json::from_value(args.clone()).context("invalid dbsnp_get_rsids arguments")?;
    let mut pending = Vec::new();
    for raw in &args.rsids {
        let item = raw.trim();
        if item.is_empty() {
            continue;
        }
        let (_number, rsid) = require_rsid(item)?;
        if !pending.iter().any(|seen: &String| seen == &rsid) {
            pending.push(rsid);
        }
    }
    if pending.is_empty() {
        bail!("provide 1 to {MAX_RSIDS} rsIDs");
    }
    if pending.len() > MAX_RSIDS {
        bail!("at most {MAX_RSIDS} unique rsIDs per call");
    }
    let started = Instant::now();
    let mut records = Vec::new();
    let mut not_found = Vec::new();
    let mut not_processed = Vec::new();
    for (index, rsid) in pending.iter().enumerate() {
        if started.elapsed() > BATCH_DEADLINE {
            not_processed.extend(pending[index..].iter().cloned());
            break;
        }
        match fetch_refsnp(bio, rsid).await? {
            None => not_found.push(rsid.clone()),
            Some(payload) => records.push(distill_refsnp(&payload)?),
        }
    }
    Ok(json!({
        "source": "NCBI dbSNP",
        "source_url": DBSNP_BROWSER,
        "n_requested": pending.len(),
        "records": records,
        "not_found": not_found,
        "not_processed": not_processed
    }))
}

pub(super) async fn search_by_region(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Region =
        serde_json::from_value(args.clone()).context("invalid dbsnp_search_by_region arguments")?;
    let chrom = normalize_chrom(&args.chrom, super::ChromKind::Dbsnp)?;
    let (start, stop) = require_region(args.start, args.stop)?;
    let assembly = args.assembly.trim();
    let field = match assembly {
        "GRCh38" => "POSITION",
        "GRCh37" => "POSITION_GRCH37",
        _ => bail!("assembly must be GRCh38 or GRCh37"),
    };
    let max_rsids = page_bound(args.max_rsids, 1, 1000, "max_rsids")?;
    let term = format!("{chrom}[CHR] AND {start}:{stop}[{field}] AND \"Homo sapiens\"[ORGN]");
    let raw = ncbi_json(
        bio,
        "esearch.fcgi",
        vec![
            ("db".into(), "snp".into()),
            ("term".into(), term.clone()),
            ("retmax".into(), max_rsids.to_string()),
            ("retstart".into(), "0".into()),
        ],
    )
    .await?;
    let result = raw
        .get("esearchresult")
        .context("dbSNP omitted search results")?;
    if result.get("ERROR").is_some() {
        bail!("dbSNP rejected the search expression");
    }
    let total = result
        .get("count")
        .and_then(json_string)
        .and_then(|text| text.parse::<u64>().ok())
        .context("dbSNP omitted the search count")?;
    let rsids: Vec<String> = result
        .get("idlist")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(json_string)
        .map(|id| format!("rs{}", id.trim_start_matches("rs").trim_start_matches("RS")))
        .collect();
    if rsids.len() > max_rsids {
        bail!("dbSNP returned inconsistent pagination");
    }
    Ok(json!({
        "source": "NCBI dbSNP",
        "source_url": DBSNP_BROWSER,
        "chrom": chrom,
        "start": start,
        "stop": stop,
        "assembly": assembly,
        "term": term,
        "total": total,
        "n_returned": rsids.len(),
        "truncated": total > rsids.len() as u64,
        "rsids": rsids
    }))
}

async fn fetch_refsnp(bio: &NativeBio, rsid: &str) -> Result<Option<Value>> {
    let number = rsid.trim_start_matches("rs");
    let url = format!("{}/refsnp/{number}", variation_base(bio));
    let response = bio.http().send(NCBI, Method::GET, &url, &[]).await?;
    if response.status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let payload = response.json()?;
    Ok(Some(payload))
}

pub(super) fn distill_refsnp(payload: &Value) -> Result<Value> {
    let number = json_string(&payload["refsnp_id"]).context("dbSNP omitted refsnp_id")?;
    let rsid = format!("rs{}", number.trim_start_matches("rs"));
    let citations: Vec<Value> = payload
        .get("citations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut base = json!({
        "rsid": rsid,
        "url": dbsnp_url(&format!("rs{}", number.trim_start_matches("rs"))),
        "create_date": payload.get("create_date"),
        "last_update_date": payload.get("last_update_date"),
        "last_update_build_id": payload.get("last_update_build_id"),
        "n_citations": citations.len(),
        "citations_pmids": citations.iter().take(MAX_CITATIONS).cloned().collect::<Vec<_>>(),
        "citations_truncated": citations.len() > MAX_CITATIONS
    });
    if let Some(merged) = payload
        .pointer("/merged_snapshot_data/merged_into")
        .and_then(Value::as_array)
        .filter(|rows| !rows.is_empty())
    {
        let into: Vec<String> = merged
            .iter()
            .filter_map(json_string)
            .map(|id| format!("rs{id}"))
            .collect();
        base["status"] = json!("merged");
        base["merged_into"] = json!(into);
        return Ok(base);
    }
    let Some(psd) = payload
        .get("primary_snapshot_data")
        .filter(|row| row.is_object())
    else {
        base["status"] = json!("no_data");
        return Ok(base);
    };
    let mane: BTreeSet<String> = payload
        .get("mane_select_ids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(json_string)
        .collect();
    let placements = assembly_placements(psd);
    let alleles = distill_alleles(psd, &mane);
    base["status"] = json!("live");
    base["variant_type"] = psd.get("variant_type").cloned().unwrap_or(Value::Null);
    base["mane_select_ids"] = json!(mane.into_iter().collect::<Vec<_>>());
    base["placements"] = json!(placements);
    base["alleles"] = json!(alleles);
    Ok(base)
}

fn assembly_placements(psd: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    for placement in psd
        .get("placements_with_allele")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let traits = placement
            .pointer("/placement_annot/seq_id_traits_by_assembly")
            .and_then(Value::as_array)
            .and_then(|rows| rows.first())
            .cloned();
        let Some(traits) = traits else {
            continue;
        };
        if traits.get("is_chromosome") != Some(&json!(true))
            && traits.get("is_chromosome").and_then(Value::as_bool) != Some(true)
        {
            continue;
        }
        let alleles: Vec<&Value> = placement
            .get("alleles")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|row| row.pointer("/allele/spdi"))
            .collect();
        if alleles.is_empty() {
            continue;
        }
        let reference = json_string(&alleles[0]["deleted_sequence"]);
        let mut alts: Vec<String> = alleles
            .iter()
            .filter_map(|spdi| {
                let inserted = json_string(&spdi["inserted_sequence"])?;
                let deleted = json_string(&spdi["deleted_sequence"]).unwrap_or_default();
                if inserted == deleted {
                    None
                } else {
                    Some(inserted)
                }
            })
            .collect();
        alts.sort();
        alts.dedup();
        let assembly_full = json_string(&traits["assembly_name"]).unwrap_or_default();
        let assembly = assembly_full.split('.').next().unwrap_or(&assembly_full);
        let seq_id = json_string(&placement["seq_id"]).unwrap_or_default();
        let position = json_i64(&alleles[0]["position"]).unwrap_or(-1) + 1;
        out.push(json!({
            "assembly": assembly,
            "assembly_full": assembly_full,
            "seq_id": seq_id,
            "chrom": chrom_from_seq(&seq_id),
            "position": position,
            "ref": reference,
            "alts": alts,
            "is_primary": placement.get("is_ptlp").and_then(Value::as_bool).unwrap_or(false)
        }));
    }
    out.sort_by(|a, b| {
        b["is_primary"]
            .as_bool()
            .unwrap_or(false)
            .cmp(&a["is_primary"].as_bool().unwrap_or(false))
            .then_with(|| {
                a["assembly"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["assembly"].as_str().unwrap_or(""))
            })
    });
    out
}

fn distill_alleles(psd: &Value, mane: &BTreeSet<String>) -> Vec<Value> {
    let primary = psd
        .get("placements_with_allele")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|row| row.get("is_ptlp").and_then(Value::as_bool) == Some(true));
    let Some(primary) = primary else {
        return Vec::new();
    };
    let annotations = psd
        .get("allele_annotations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut alleles = Vec::new();
    for (index, entry) in primary
        .get("alleles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let Some(spdi) = entry.pointer("/allele/spdi") else {
            continue;
        };
        let deleted = json_string(&spdi["deleted_sequence"]).unwrap_or_default();
        let inserted = json_string(&spdi["inserted_sequence"]).unwrap_or_default();
        if deleted == inserted {
            continue;
        }
        let annotation = annotations.get(index).cloned().unwrap_or(json!({}));
        alleles.push(json!({
            "allele": inserted,
            "ref": deleted,
            "spdi": spdi_str(spdi),
            "hgvs": entry.get("hgvs"),
            "frequencies": frequencies(&annotation),
            "clinvar": clinvar_xrefs(&annotation),
            "genes": genes(&annotation, mane)
        }));
    }
    alleles
}

fn frequencies(annotation: &Value) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    for freq in annotation
        .get("frequency")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let study = json_string(&freq["study_name"]).unwrap_or_default();
        let version = freq.get("study_version").cloned().unwrap_or(Value::Null);
        let key = format!("{study}:{version}");
        if !seen.insert(key) {
            continue;
        }
        let ac = json_i64(&freq["allele_count"]);
        let tc = json_i64(&freq["total_count"]);
        let af = match (ac, tc) {
            (Some(ac), Some(tc)) if tc > 0 => Some((ac as f64) / (tc as f64)),
            _ => None,
        };
        rows.push(json!({
            "study": freq.get("study_name"),
            "study_version": version,
            "allele_count": freq.get("allele_count"),
            "total_count": freq.get("total_count"),
            "af": af
        }));
    }
    rows.sort_by(|a, b| {
        a["study"]
            .as_str()
            .unwrap_or("")
            .cmp(b["study"].as_str().unwrap_or(""))
    });
    rows
}

fn clinvar_xrefs(annotation: &Value) -> Vec<Value> {
    let mut rows: Vec<Value> = annotation
        .get("clinical")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|row| {
            json!({
                "rcv_accession": row.get("accession_version"),
                "clinical_significances": row.get("clinical_significances").cloned().unwrap_or(json!([])),
                "review_status": row.get("review_status"),
                "last_evaluated_date": row.get("last_evaluated_date"),
                "disease_names": row.get("disease_names").cloned().unwrap_or(json!([]))
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        a["rcv_accession"]
            .as_str()
            .unwrap_or("")
            .cmp(b["rcv_accession"].as_str().unwrap_or(""))
    });
    rows
}

fn genes(annotation: &Value, mane: &BTreeSet<String>) -> Vec<Value> {
    let mut out = Vec::new();
    for asm in annotation
        .get("assembly_annotation")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for gene in asm
            .get("genes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let mut consequences = BTreeSet::new();
            let mut mane_select = Vec::new();
            for rna in gene
                .get("rnas")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                for so in rna
                    .get("sequence_ontology")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(name) = json_string(&so["name"]) {
                        consequences.insert(name);
                    }
                }
                let protein = rna.get("protein").cloned().unwrap_or(json!({}));
                for so in protein
                    .get("sequence_ontology")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(name) = json_string(&so["name"]) {
                        consequences.insert(name);
                    }
                }
                if json_string(&rna["id"]).is_some_and(|id| mane.contains(&id)) {
                    let pv = protein.pointer("/variant/spdi");
                    mane_select.push(json!({
                        "transcript_hgvs": rna.get("hgvs"),
                        "protein_spdi": pv.map(spdi_str)
                    }));
                }
            }
            out.push(json!({
                "symbol": gene.get("locus"),
                "gene_id": gene.get("id"),
                "name": gene.get("name"),
                "orientation": gene.get("orientation"),
                "consequences": consequences.into_iter().collect::<Vec<_>>(),
                "mane_select": mane_select
            }));
        }
    }
    out.sort_by(|a, b| {
        a["symbol"]
            .as_str()
            .unwrap_or("")
            .cmp(b["symbol"].as_str().unwrap_or(""))
    });
    out
}

fn spdi_str(spdi: &Value) -> String {
    format!(
        "{}:{}:{}:{}",
        json_string(&spdi["seq_id"]).unwrap_or_default(),
        json_string(&spdi["position"]).unwrap_or_else(|| json_i64(&spdi["position"])
            .map(|n| n.to_string())
            .unwrap_or_default()),
        json_string(&spdi["deleted_sequence"]).unwrap_or_default(),
        json_string(&spdi["inserted_sequence"]).unwrap_or_default()
    )
}

fn chrom_from_seq(seq_id: &str) -> Value {
    let acc = seq_id.split('.').next().unwrap_or(seq_id);
    match acc {
        "NC_000023" => json!("X"),
        "NC_000024" => json!("Y"),
        "NC_012920" => json!("MT"),
        other => other
            .strip_prefix("NC_")
            .and_then(|digits| digits.parse::<u32>().ok())
            .filter(|n| (1..=22).contains(n))
            .map(|n| json!(n.to_string()))
            .unwrap_or(Value::Null),
    }
}
