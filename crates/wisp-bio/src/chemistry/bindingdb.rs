use super::{
    as_object_array, cap, join_url, json_plain, require_range, require_text, send, NativeBio,
    BINDINGDB,
};
use anyhow::{bail, Context, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};

pub async fn ligands_by_target(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Ligands = serde_json::from_value(args.clone())
        .context("invalid bindingdb_ligands_by_target arguments")?;
    let uniprot = normalize_uniprot(&args.uniprot)?;
    if !(1..=10_000_000).contains(&args.affinity_cutoff_nm) {
        bail!("affinity_cutoff_nm must be between 1 and 10000000");
    }
    let max_rows = require_range(args.max_rows, 1, 1000, "max_rows")?;
    let params = vec![
        ("uniprot".into(), uniprot.clone()),
        ("cutoff".into(), args.affinity_cutoff_nm.to_string()),
        ("response".into(), "application/json".into()),
    ];
    let url = join_url(&super::bindingdb_base(bio), "getLigandsByUniprots");
    let root = bindingdb_json(bio, &url, &params).await?;
    let mut rows = Vec::new();
    for raw in as_object_array(root.get("affinities")) {
        rows.push(json!({
            "target_name": cell(raw, &["query", "target"]),
            "monomer_id": cell(raw, &["monomerid", "monomer_id"]),
            "smiles": cell(raw, &["smile", "smiles"]),
            "affinity_type": cell(raw, &["affinity_type"]),
            "affinity": cell(raw, &["affinity"]),
            "pmid": null_if_empty(cell(raw, &["pmid"])),
            "doi": null_if_empty(cell(raw, &["doi"])),
            "url": monomer_url(&cell(raw, &["monomerid", "monomer_id"]))
        }));
    }
    rows.sort_by(|a, b| {
        json_plain(&a["affinity_type"])
            .cmp(&json_plain(&b["affinity_type"]))
            .then(
                affinity_sort_key(&json_plain(&a["affinity"]).unwrap_or_default()).total_cmp(
                    &affinity_sort_key(&json_plain(&b["affinity"]).unwrap_or_default()),
                ),
            )
            .then(json_plain(&a["monomer_id"]).cmp(&json_plain(&b["monomer_id"])))
    });
    let total = rows.len();
    let (page, truncated) = cap(&rows, max_rows);
    Ok(json!({
        "source": "BindingDB",
        "uniprot": uniprot,
        "affinity_cutoff_nm": args.affinity_cutoff_nm,
        "n_rows_total": total,
        "truncated": truncated,
        "rows": page
    }))
}

pub async fn targets_by_compound(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Targets = serde_json::from_value(args.clone())
        .context("invalid bindingdb_targets_by_compound arguments")?;
    let smiles = require_text(&args.smiles, "smiles", 8192)?;
    if !(0.5..=1.0).contains(&args.similarity) {
        bail!("similarity must be between 0.5 and 1.0");
    }
    let max_rows = require_range(args.max_rows, 1, 1000, "max_rows")?;
    let params = vec![
        ("smiles".into(), smiles.to_string()),
        ("cutoff".into(), args.similarity.to_string()),
        ("response".into(), "application/json".into()),
    ];
    let url = join_url(&super::bindingdb_base(bio), "getTargetByCompound");
    let root = bindingdb_json(bio, &url, &params).await?;
    let mut rows = Vec::new();
    let affinities = root
        .get("bdb.affinities")
        .or_else(|| root.get("affinities"));
    for raw in as_object_array(affinities) {
        rows.push(json!({
            "monomer_id": cell(raw, &["monomerid", "monomer_id"]),
            "smiles": cell(raw, &["smiles", "smile"]),
            "ligand_name": cell(raw, &["inhibitor", "ligand_name"]),
            "target_name": cell(raw, &["target", "target_name"]),
            "species": cell(raw, &["species"]),
            "affinity_type": cell(raw, &["affinity_type"]),
            "affinity": cell(raw, &["affinity"]),
            "tanimoto": null_if_empty(cell(raw, &["tanimoto"])),
            "url": monomer_url(&cell(raw, &["monomerid", "monomer_id"]))
        }));
    }
    rows.sort_by(|a, b| {
        json_plain(&a["target_name"])
            .cmp(&json_plain(&b["target_name"]))
            .then(json_plain(&a["affinity_type"]).cmp(&json_plain(&b["affinity_type"])))
            .then(
                affinity_sort_key(&json_plain(&a["affinity"]).unwrap_or_default()).total_cmp(
                    &affinity_sort_key(&json_plain(&b["affinity"]).unwrap_or_default()),
                ),
            )
            .then(json_plain(&a["monomer_id"]).cmp(&json_plain(&b["monomer_id"])))
    });
    let hit = root
        .get("bdb.hit")
        .or_else(|| root.get("hit"))
        .and_then(|value| json_plain(value)?.parse::<u64>().ok());
    let total = rows.len();
    let (page, truncated) = cap(&rows, max_rows);
    Ok(json!({
        "source": "BindingDB",
        "smiles": smiles,
        "similarity": args.similarity,
        "api_hit_count": hit,
        "n_rows_total": total,
        "truncated": truncated,
        "rows": page
    }))
}

async fn bindingdb_json(bio: &NativeBio, url: &str, params: &[(String, String)]) -> Result<Value> {
    let response = send(bio, BINDINGDB, Method::GET, url, params).await?;
    response.check()?;
    if response.body.iter().all(|byte| byte.is_ascii_whitespace()) {
        // Official docs: no match returns an empty body, not an HTTP error.
        return Ok(json!({}));
    }
    let raw: Value =
        serde_json::from_slice(&response.body).context("upstream returned invalid JSON")?;
    unwrap_root(raw)
}

fn unwrap_root(raw: Value) -> Result<Value> {
    if raw.get("affinities").is_some()
        || raw.get("bdb.affinities").is_some()
        || raw.get("bdb.hit").is_some()
    {
        return Ok(raw);
    }
    match raw {
        Value::Object(map) if map.is_empty() => Ok(json!({})),
        Value::Object(map) if map.len() == 1 => Ok(map.into_iter().next().unwrap().1),
        _ => bail!("BindingDB returned an unexpected JSON shape"),
    }
}

fn cell(row: &Value, names: &[&str]) -> Value {
    for name in names {
        let prefixed = format!("bdb.{name}");
        for key in [*name, prefixed.as_str()] {
            if let Some(value) = row.get(key) {
                if let Some(text) = json_plain(value) {
                    return json!(text);
                }
            }
        }
    }
    Value::Null
}

fn null_if_empty(value: Value) -> Value {
    match &value {
        Value::String(text) if text.is_empty() => Value::Null,
        Value::Null => Value::Null,
        _ => value,
    }
}

fn monomer_url(monomer: &Value) -> Value {
    match json_plain(monomer) {
        Some(id) => json!(format!(
            "https://www.bindingdb.org/rwd/bind/chemsearch/marvin/MolStructure.jsp?monomerid={id}"
        )),
        None => Value::Null,
    }
}

fn affinity_sort_key(affinity: &str) -> f64 {
    let digits: String = affinity
        .chars()
        .skip_while(|ch| !ch.is_ascii_digit() && *ch != '.')
        .take_while(|ch| {
            ch.is_ascii_digit()
                || *ch == '.'
                || *ch == 'e'
                || *ch == 'E'
                || *ch == '+'
                || *ch == '-'
        })
        .collect();
    digits.parse().unwrap_or(f64::INFINITY)
}

fn normalize_uniprot(value: &str) -> Result<String> {
    let value = require_text(value, "uniprot", 15)?.to_ascii_uppercase();
    if !is_uniprot_accession(&value) {
        bail!("uniprot must be a UniProtKB accession");
    }
    Ok(value)
}

fn is_uniprot_accession(value: &str) -> bool {
    let b = value.as_bytes();
    let digit = |i: usize| b[i].is_ascii_digit();
    let upper = |i: usize| b[i].is_ascii_uppercase();
    let an = |i: usize| b[i].is_ascii_uppercase() || b[i].is_ascii_digit();
    let start_anrz = |c: u8| (b'A'..=b'N').contains(&c) || (b'R'..=b'Z').contains(&c);
    match b.len() {
        6 if matches!(b[0], b'O' | b'P' | b'Q') => digit(1) && an(2) && an(3) && an(4) && digit(5),
        6 if start_anrz(b[0]) => digit(1) && upper(2) && an(3) && an(4) && digit(5),
        10 if start_anrz(b[0]) => {
            digit(1)
                && upper(2)
                && an(3)
                && an(4)
                && digit(5)
                && upper(6)
                && an(7)
                && an(8)
                && digit(9)
        }
        _ => false,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ligands {
    uniprot: String,
    #[serde(default = "default_cutoff")]
    affinity_cutoff_nm: u64,
    #[serde(default = "default_max_rows")]
    max_rows: usize,
}

fn default_cutoff() -> u64 {
    10_000
}
fn default_max_rows() -> usize {
    100
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Targets {
    smiles: String,
    #[serde(default = "default_similarity")]
    similarity: f64,
    #[serde(default = "default_max_rows")]
    max_rows: usize,
}

fn default_similarity() -> f64 {
    0.85
}
