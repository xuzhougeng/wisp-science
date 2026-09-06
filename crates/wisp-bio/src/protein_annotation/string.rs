use super::{
    bound_page, default_max, default_network_max, default_network_type, default_score,
    default_species, json_f64, post_json, require_ids, string_base, taxon_id, text_field, Fetch,
    MAX_IDS, STRING, STRING_SITE,
};
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

const CALLER: &str = "wisp-science";
const EVIDENCE: &[&str] = &[
    "nscore", "fscore", "pscore", "ascore", "escore", "dscore", "tscore",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapArgs {
    pub symbols: Vec<String>,
    #[serde(default = "default_species")]
    pub species: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NetworkArgs {
    symbols: Vec<String>,
    #[serde(default = "default_species")]
    species: i64,
    #[serde(default = "default_score")]
    required_score: i64,
    #[serde(default = "default_network_type")]
    network_type: String,
    #[serde(default = "default_network_max")]
    max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HomologyArgs {
    symbols: Vec<String>,
    #[serde(default = "default_species")]
    species: i64,
    #[serde(default = "default_max")]
    max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BestHitArgs {
    symbols: Vec<String>,
    #[serde(default = "default_species")]
    species: i64,
    target_species: Option<i64>,
    #[serde(default = "default_max")]
    max_results: u32,
}

struct Mapping {
    version: Value,
    mapped: Vec<Value>,
    unmapped: Vec<String>,
}

pub(super) async fn map_string_ids(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: MapArgs =
        serde_json::from_value(args.clone()).context("invalid STRING identifier arguments")?;
    let mapping = map_symbols(bio, &args.symbols, args.species).await?;
    Ok(json!({
        "source": "STRING",
        "source_url": STRING_SITE,
        "string_version": mapping.version,
        "species": args.species,
        "query": {"symbols": require_ids(&args.symbols, MAX_IDS, "symbol")?},
        "mapped": mapping.mapped,
        "unmapped": mapping.unmapped
    }))
}

pub(super) async fn get_string_network(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: NetworkArgs =
        serde_json::from_value(args.clone()).context("invalid STRING network arguments")?;
    if !(0..=1000).contains(&args.required_score) {
        bail!("required_score must be between 0 and 1000");
    }
    let network_type = match args.network_type.trim() {
        "functional" | "physical" => args.network_type.trim(),
        other => bail!("{other:?} is not a STRING network_type (functional or physical)"),
    };
    let cap = bound_page(args.max_results)?;
    let mapping = map_symbols(bio, &args.symbols, args.species).await?;
    let string_ids: Vec<String> = mapping
        .mapped
        .iter()
        .filter_map(|row| {
            row.get("string_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    let mut edges = if string_ids.is_empty() {
        Vec::new()
    } else {
        let payload = string_call(
            bio,
            "network",
            vec![
                ("identifiers".into(), string_ids.join("\r")),
                ("species".into(), args.species.to_string()),
                ("required_score".into(), args.required_score.to_string()),
                ("network_type".into(), network_type.to_string()),
                ("add_nodes".into(), "0".into()),
            ],
        )
        .await?;
        canonical_edges(&expect_array(payload, "network")?)?
    };
    let total = edges.len();
    let truncated = edges.len() > cap;
    edges.truncate(cap);
    let nodes = node_table(&mapping.mapped, &edges);
    Ok(json!({
        "source": "STRING",
        "source_url": STRING_SITE,
        "string_version": mapping.version,
        "query": {
            "symbols": require_ids(&args.symbols, MAX_IDS, "symbol")?,
            "species": args.species,
            "required_score": args.required_score,
            "network_type": network_type,
            "add_nodes": 0,
            "max_results": cap
        },
        "nodes": nodes,
        "unmapped": mapping.unmapped,
        "total_edges": total,
        "returned": edges.len(),
        "has_more": truncated,
        "edges": edges
    }))
}

pub(super) async fn get_string_similarity_scores(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: HomologyArgs =
        serde_json::from_value(args.clone()).context("invalid STRING homology arguments")?;
    let cap = bound_page(args.max_results)?;
    let mapping = map_symbols(bio, &args.symbols, args.species).await?;
    let string_ids: Vec<String> = mapping
        .mapped
        .iter()
        .filter_map(|row| {
            row.get("string_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    let names: HashMap<String, String> = mapping
        .mapped
        .iter()
        .filter_map(|row| {
            Some((
                row.get("string_id")?.as_str()?.to_string(),
                row.get("preferred_name")?.as_str()?.to_string(),
            ))
        })
        .collect();
    let mut pairs = if string_ids.is_empty() {
        Vec::new()
    } else {
        let payload = string_call(
            bio,
            "homology",
            vec![
                ("identifiers".into(), string_ids.join("\r")),
                ("species".into(), args.species.to_string()),
            ],
        )
        .await?;
        parse_homology(&expect_array(payload, "homology")?, &names)?
    };
    let total = pairs.len();
    pairs.truncate(cap);
    Ok(json!({
        "source": "STRING",
        "source_url": STRING_SITE,
        "string_version": mapping.version,
        "species": args.species,
        "mapped": mapping.mapped,
        "unmapped": mapping.unmapped,
        "total_pairs": total,
        "returned": pairs.len(),
        "has_more": total > pairs.len(),
        "pairs": pairs
    }))
}

pub(super) async fn get_string_best_similarity_hits(
    bio: &NativeBio,
    args: &Value,
) -> Result<Value> {
    let args: BestHitArgs =
        serde_json::from_value(args.clone()).context("invalid STRING homology_best arguments")?;
    let cap = bound_page(args.max_results)?;
    if let Some(target) = args.target_species {
        taxon_id(target, "target_species")?;
    }
    let mapping = map_symbols(bio, &args.symbols, args.species).await?;
    let string_ids: Vec<String> = mapping
        .mapped
        .iter()
        .filter_map(|row| {
            row.get("string_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    let names: HashMap<String, String> = mapping
        .mapped
        .iter()
        .filter_map(|row| {
            Some((
                row.get("string_id")?.as_str()?.to_string(),
                row.get("preferred_name")?.as_str()?.to_string(),
            ))
        })
        .collect();
    let mut hits = if string_ids.is_empty() {
        Vec::new()
    } else {
        let mut params = vec![
            ("identifiers".into(), string_ids.join("\r")),
            ("species".into(), args.species.to_string()),
        ];
        if let Some(target) = args.target_species {
            params.push(("species_b".into(), target.to_string()));
        }
        let payload = string_call(bio, "homology_best", params).await?;
        parse_best_hits(&expect_array(payload, "homology_best")?, &names)?
    };
    let total = hits.len();
    hits.truncate(cap);
    Ok(json!({
        "source": "STRING",
        "source_url": STRING_SITE,
        "string_version": mapping.version,
        "species": args.species,
        "target_species": args.target_species,
        "mapped": mapping.mapped,
        "unmapped": mapping.unmapped,
        "total_hits": total,
        "returned": hits.len(),
        "has_more": total > hits.len(),
        "hits": hits
    }))
}

async fn map_symbols(bio: &NativeBio, symbols: &[String], species: i64) -> Result<Mapping> {
    let symbols = require_ids(symbols, MAX_IDS, "symbol")?;
    taxon_id(species, "species")?;
    let version = string_version(bio).await?;
    let payload = match string_try(
        bio,
        "get_string_ids",
        vec![
            ("identifiers".into(), symbols.join("\r")),
            ("species".into(), species.to_string()),
            ("limit".into(), "1".into()),
            ("echo_query".into(), "1".into()),
        ],
    )
    .await?
    {
        None => {
            return Ok(Mapping {
                version,
                mapped: Vec::new(),
                unmapped: symbols,
            });
        }
        Some(value) => value,
    };
    let rows = expect_array(payload, "get_string_ids")?;
    let (mapped, unmapped) = split_mapping(&symbols, &rows)?;
    Ok(Mapping {
        version,
        mapped,
        unmapped,
    })
}

async fn string_version(bio: &NativeBio) -> Result<Value> {
    let payload = string_call(bio, "version", Vec::new()).await?;
    match payload {
        Value::Array(rows) => Ok(rows.into_iter().next().unwrap_or(Value::Null)),
        other => Ok(other),
    }
}

async fn string_call(
    bio: &NativeBio,
    method: &str,
    params: Vec<(String, String)>,
) -> Result<Value> {
    match string_try(bio, method, params).await? {
        Some(value) => Ok(value),
        None => bail!("STRING returned HTTP 404 for {method}"),
    }
}

async fn string_try(
    bio: &NativeBio,
    method: &str,
    mut params: Vec<(String, String)>,
) -> Result<Option<Value>> {
    params.push(("caller_identity".into(), CALLER.into()));
    let url = format!("{}/json/{method}", string_base(bio));
    match post_json(bio, STRING, &url, &params).await? {
        Fetch::Json(value) => Ok(Some(value)),
        Fetch::Empty => Ok(Some(json!([]))),
        Fetch::NotFound => Ok(None),
    }
}

fn expect_array(value: Value, method: &str) -> Result<Vec<Value>> {
    match value {
        Value::Array(rows) => Ok(rows),
        _ => bail!("STRING {method} did not return a JSON array"),
    }
}

fn split_mapping(symbols: &[String], rows: &[Value]) -> Result<(Vec<Value>, Vec<String>)> {
    let mut by_index: HashMap<i64, &Value> = HashMap::new();
    for row in rows {
        if let Some(idx) = json_index(row) {
            by_index.entry(idx).or_insert(row);
        }
    }
    let mut mapped = Vec::new();
    let mut unmapped = Vec::new();
    for (i, symbol) in symbols.iter().enumerate() {
        let row = by_index.get(&(i as i64)).copied().or_else(|| {
            rows.iter().find(|row| {
                row.get("queryItem")
                    .and_then(Value::as_str)
                    .is_some_and(|item| item == symbol)
            })
        });
        match row {
            Some(row) => {
                let string_id =
                    text_field(row, "stringId").context("STRING mapping row omitted stringId")?;
                mapped.push(json!({
                    "query": symbol,
                    "string_id": string_id,
                    "preferred_name": text_field(row, "preferredName"),
                    "ncbi_taxon_id": row.get("ncbiTaxonId").cloned().unwrap_or(Value::Null),
                    "annotation": text_field(row, "annotation"),
                    "url": format!("{STRING_SITE}/network/{string_id}")
                }));
            }
            None => unmapped.push(symbol.clone()),
        }
    }
    Ok((mapped, unmapped))
}

fn json_index(row: &Value) -> Option<i64> {
    match row.get("queryIndex") {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_u64().map(|n| n as i64)),
        Some(Value::String(text)) => text.parse().ok(),
        _ => None,
    }
}

fn canonical_edges(rows: &[Value]) -> Result<Vec<Value>> {
    let mut best: BTreeMap<(String, String), Value> = BTreeMap::new();
    for row in rows {
        let id_a = text_field(row, "stringId_A").context("network row omitted stringId_A")?;
        let id_b = text_field(row, "stringId_B").context("network row omitted stringId_B")?;
        let name_a = text_field(row, "preferredName_A").unwrap_or_else(|| id_a.clone());
        let name_b = text_field(row, "preferredName_B").unwrap_or_else(|| id_b.clone());
        let score = round3(json_f64(row.get("score").unwrap_or(&Value::Null))?);
        let mut evidence = serde_json::Map::new();
        for channel in EVIDENCE {
            if let Some(raw) = row.get(*channel) {
                let value = round3(json_f64(raw)?);
                if value > 0.0 {
                    evidence.insert((*channel).to_string(), json!(value));
                }
            }
        }
        let (left_name, left_id, right_name, right_id) =
            if (name_a.as_str(), id_a.as_str()) <= (name_b.as_str(), id_b.as_str()) {
                (name_a, id_a, name_b, id_b)
            } else {
                (name_b, id_b, name_a, id_a)
            };
        let key = if left_id <= right_id {
            (left_id.clone(), right_id.clone())
        } else {
            (right_id.clone(), left_id.clone())
        };
        let edge = json!({
            "a": left_name,
            "b": right_name,
            "id_a": left_id,
            "id_b": right_id,
            "score": score,
            "evidence": evidence
        });
        match best.get(&key) {
            Some(existing)
                if existing.get("score").and_then(Value::as_f64).unwrap_or(0.0) >= score => {}
            _ => {
                best.insert(key, edge);
            }
        }
    }
    Ok(best.into_values().collect())
}

fn node_table(mapped: &[Value], edges: &[Value]) -> Vec<Value> {
    let mut degree: HashMap<String, u32> = mapped
        .iter()
        .filter_map(|row| row.get("preferred_name").and_then(Value::as_str))
        .map(|name| (name.to_string(), 0))
        .collect();
    for edge in edges {
        for key in ["a", "b"] {
            if let Some(name) = edge.get(key).and_then(Value::as_str) {
                *degree.entry(name.to_string()).or_insert(0) += 1;
            }
        }
    }
    let mut nodes: Vec<Value> = mapped
        .iter()
        .map(|row| {
            let name = row
                .get("preferred_name")
                .and_then(Value::as_str)
                .unwrap_or("");
            json!({
                "query": row.get("query"),
                "name": name,
                "string_id": row.get("string_id"),
                "degree": degree.get(name).copied().unwrap_or(0),
                "url": row.get("url")
            })
        })
        .collect();
    nodes.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["name"].as_str().unwrap_or(""))
            .then_with(|| {
                a["string_id"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["string_id"].as_str().unwrap_or(""))
            })
    });
    nodes
}

fn parse_homology(rows: &[Value], names: &HashMap<String, String>) -> Result<Vec<Value>> {
    let mut seen: BTreeMap<(String, String), Value> = BTreeMap::new();
    for row in rows {
        let mut id_a = text_field(row, "stringId_A").context("homology row omitted stringId_A")?;
        let mut id_b = text_field(row, "stringId_B").context("homology row omitted stringId_B")?;
        let mut tax_a = row.get("ncbiTaxonId_A").cloned().unwrap_or(Value::Null);
        let mut tax_b = row.get("ncbiTaxonId_B").cloned().unwrap_or(Value::Null);
        let bitscore = json_f64(row.get("bitscore").unwrap_or(&Value::Null))?;
        if (id_b.as_str(), tax_key(&tax_b)) < (id_a.as_str(), tax_key(&tax_a)) {
            std::mem::swap(&mut id_a, &mut id_b);
            std::mem::swap(&mut tax_a, &mut tax_b);
        }
        let key = (id_a.clone(), id_b.clone());
        if let Some(existing) = seen.get(&key) {
            let prev = existing
                .get("bitscore")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            if (prev - bitscore).abs() > 0.05 {
                bail!("STRING homology bitscores disagreed for {id_a}–{id_b}");
            }
            continue;
        }
        seen.insert(
            key,
            json!({
                "id_a": id_a,
                "id_b": id_b,
                "name_a": names.get(&id_a),
                "name_b": names.get(&id_b),
                "taxon_a": tax_a,
                "taxon_b": tax_b,
                "bitscore": bitscore,
                "self": id_a == id_b
            }),
        );
    }
    Ok(seen.into_values().collect())
}

fn parse_best_hits(rows: &[Value], names: &HashMap<String, String>) -> Result<Vec<Value>> {
    let mut hits: Vec<Value> = rows
        .iter()
        .map(|row| {
            let query_id =
                text_field(row, "stringId_A").context("homology_best omitted stringId_A")?;
            Ok(json!({
                "query_id": query_id,
                "query_name": names.get(&query_id),
                "query_taxon": row.get("ncbiTaxonId_A").cloned().unwrap_or(Value::Null),
                "hit_id": text_field(row, "stringId_B"),
                "hit_taxon": row.get("ncbiTaxonId_B").cloned().unwrap_or(Value::Null),
                "bitscore": json_f64(row.get("bitscore").unwrap_or(&Value::Null))?,
                "url": text_field(row, "stringId_B").map(|id| format!("{STRING_SITE}/network/{id}"))
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    hits.sort_by(|a, b| {
        a["query_id"]
            .as_str()
            .unwrap_or("")
            .cmp(b["query_id"].as_str().unwrap_or(""))
            .then_with(|| {
                a["hit_id"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["hit_id"].as_str().unwrap_or(""))
            })
    });
    Ok(hits)
}

fn tax_key(value: &Value) -> i64 {
    match value {
        Value::Number(number) => number.as_i64().unwrap_or(0),
        Value::String(text) => text.parse().unwrap_or(0),
        _ => 0,
    }
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}
