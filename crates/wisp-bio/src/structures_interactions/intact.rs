use super::{
    api_base, as_f64, bound_int, bound_score, json_string, listify, object_field, path_segment,
    require_ok, require_text, send_json, text_field, unique_ids, INTACT, INTACT_DEFAULT,
    INTACT_SITE,
};
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

const PAGE_SIZE: usize = 100;
const MAX_SWEEP: usize = 500;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fetch {
    query: String,
    #[serde(default)]
    min_mi_score: f64,
    #[serde(default = "default_max_score")]
    max_mi_score: f64,
    interactor_species: Option<Vec<String>>,
    #[serde(default = "default_max_records")]
    max_records_returned: u32,
}

fn default_max_score() -> f64 {
    1.0
}

fn default_max_records() -> u32 {
    200
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Interactor {
    query: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Details {
    interaction_ac: String,
    #[serde(default = "default_true")]
    include_participants: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Network {
    seed_accessions: Vec<String>,
    #[serde(default = "default_network_score")]
    min_mi_score: f64,
    #[serde(default = "default_expand")]
    max_interactors_expanded: u32,
    interactor_species: Option<Vec<String>>,
}

fn default_network_score() -> f64 {
    0.45
}

fn default_expand() -> u32 {
    10
}

pub(crate) async fn fetch_interactions(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Fetch = serde_json::from_value(args.clone())
        .context("invalid IntAct interaction search arguments")?;
    let query = require_text(&args.query, "query", 512)?;
    let min_score = bound_score(args.min_mi_score, "min_mi_score")?;
    let max_score = bound_score(args.max_mi_score, "max_mi_score")?;
    if min_score > max_score {
        bail!("min_mi_score must be ≤ max_mi_score");
    }
    let cap = bound_int(args.max_records_returned, 1, 500, "max_records_returned")?;
    let species = species_list(args.interactor_species.as_deref())?;
    let sweep = sweep_interactions(bio, &query, min_score, max_score, &species, cap).await?;
    Ok(json!({
        "source": "IntAct",
        "source_url": INTACT_SITE,
        "query": query,
        "min_mi_score": min_score,
        "max_mi_score": max_score,
        "interactor_species": species,
        "total_elements": sweep.total,
        "n_records": sweep.total,
        "returned": sweep.records.len(),
        "truncated": sweep.truncated,
        "records": sweep.records
    }))
}

pub(crate) async fn get_interactor(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Interactor =
        serde_json::from_value(args.clone()).context("invalid IntAct interactor arguments")?;
    let query = require_text(&args.query, "query", 64)?;
    let mut matches = resolve_interactors(bio, &query).await?;
    matches.sort_by(|a, b| {
        a.get("interactor_ac")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(b.get("interactor_ac").and_then(Value::as_str).unwrap_or(""))
    });
    Ok(json!({
        "source": "IntAct",
        "source_url": INTACT_SITE,
        "query": query,
        "n_matches": matches.len(),
        "interactors": matches
    }))
}

pub(crate) async fn get_interaction_details(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Details = serde_json::from_value(args.clone())
        .context("invalid IntAct interaction detail arguments")?;
    let ac = require_text(&args.interaction_ac, "interaction_ac", 32)?;
    if !ac.to_ascii_uppercase().starts_with("EBI-") {
        bail!("interaction_ac must be an IntAct accession such as EBI-15635490");
    }
    let base = api_base(bio, "INTACT_URL", INTACT_DEFAULT);
    let url = format!("{base}/graph/interaction/details/{}", path_segment(&ac));
    let (status, body) = send_json(bio, INTACT, Method::GET, &url, &[]).await?;
    if status == StatusCode::NOT_FOUND || body.is_none() {
        return Ok(json!({
            "source": "IntAct",
            "source_url": INTACT_SITE,
            "interaction_ac": ac,
            "url": interaction_url(&ac),
            "error": "not_found"
        }));
    }
    require_ok(INTACT, status)?;
    let raw = body.unwrap();
    let mut record = parse_interaction_details(&raw);
    record["source"] = json!("IntAct");
    record["source_url"] = json!(INTACT_SITE);
    record["url"] = json!(interaction_url(&ac));
    if args.include_participants {
        let participants = fetch_participants(bio, &ac).await?;
        record["n_participants"] = json!(participants.len());
        record["participants"] = json!(participants);
    }
    Ok(record)
}

pub(crate) async fn build_network(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Network =
        serde_json::from_value(args.clone()).context("invalid IntAct network arguments")?;
    let ids = unique_ids(&args.seed_accessions, 5, "seed accession", |raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            Ok(None)
        } else if trimmed.len() > 32 {
            bail!("seed accession exceeds 32 characters");
        } else {
            Ok(Some(trimmed.to_ascii_uppercase()))
        }
    })?;
    let min_score = bound_score(args.min_mi_score, "min_mi_score")?;
    let expand_cap = bound_int(
        args.max_interactors_expanded,
        0,
        25,
        "max_interactors_expanded",
    )?;
    let species = species_list(args.interactor_species.as_deref())?;
    let mut edges: BTreeMap<(String, String), Value> = BTreeMap::new();
    let mut node_ids: HashSet<String> = ids.unique.iter().cloned().collect();
    let mut partner_degree: BTreeMap<String, i64> = BTreeMap::new();
    let mut seed_sweeps = serde_json::Map::new();
    for seed in &ids.unique {
        let sweep = sweep_interactions(bio, seed, min_score, 1.0, &species, MAX_SWEEP).await?;
        seed_sweeps.insert(
            seed.clone(),
            json!({
                "total_elements": sweep.total,
                "returned": sweep.records.len(),
                "truncated": sweep.truncated
            }),
        );
        for rec in sweep.records {
            ingest_edge(&mut edges, &rec, "seed_sweep");
            for key in ["id_a", "id_b"] {
                if let Some(pid) = rec.get(key).and_then(Value::as_str) {
                    if !ids.unique.iter().any(|s| s == pid) {
                        node_ids.insert(pid.to_string());
                        *partner_degree.entry(pid.to_string()).or_insert(0) += 1;
                    }
                }
            }
        }
    }
    let mut expansion_order: Vec<String> = partner_degree.keys().cloned().collect();
    expansion_order.sort_by(|a, b| {
        partner_degree[b]
            .cmp(&partner_degree[a])
            .then_with(|| a.cmp(b))
    });
    let expanded: Vec<String> = expansion_order.iter().take(expand_cap).cloned().collect();
    let not_expanded: Vec<String> = expansion_order.iter().skip(expand_cap).cloned().collect();
    for partner in &expanded {
        let sweep = sweep_interactions(bio, partner, min_score, 1.0, &species, MAX_SWEEP).await?;
        for rec in sweep.records {
            let a = rec.get("id_a").and_then(Value::as_str).unwrap_or("");
            let b = rec.get("id_b").and_then(Value::as_str).unwrap_or("");
            if node_ids.contains(a) && node_ids.contains(b) {
                ingest_edge(&mut edges, &rec, "partner_expansion");
            }
        }
    }
    let mut edge_list: Vec<Value> = edges.into_values().collect();
    sort_records(&mut edge_list);
    let mut nodes: Vec<String> = node_ids.into_iter().collect();
    nodes.sort();
    Ok(json!({
        "source": "IntAct",
        "source_url": INTACT_SITE,
        "seeds": ids.unique,
        "min_mi_score": min_score,
        "n_nodes": nodes.len(),
        "nodes": nodes,
        "n_edges": edge_list.len(),
        "edges": edge_list,
        "seed_sweeps": seed_sweeps,
        "expansion": {
            "max_interactors_expanded": expand_cap,
            "n_partners": expansion_order.len(),
            "expanded": expanded,
            "not_expanded": not_expanded,
            "complete": not_expanded.is_empty()
        }
    }))
}

struct Sweep {
    total: u64,
    truncated: bool,
    records: Vec<Value>,
}

async fn sweep_interactions(
    bio: &NativeBio,
    query: &str,
    min_score: f64,
    max_score: f64,
    species: &[String],
    cap: usize,
) -> Result<Sweep> {
    let base = api_base(bio, "INTACT_URL", INTACT_DEFAULT);
    let url = format!("{base}/interaction/findInteractionWithFacet");
    let mut records = Vec::new();
    let mut total = 0u64;
    let mut page = 0u32;
    loop {
        let mut params = vec![
            ("query".into(), query.to_string()),
            ("minMIScore".into(), min_score.to_string()),
            ("maxMIScore".into(), max_score.to_string()),
            ("page".into(), page.to_string()),
            ("pageSize".into(), PAGE_SIZE.to_string()),
        ];
        for sp in species {
            params.push(("interactorSpeciesFilter".into(), sp.clone()));
        }
        let (status, body) = send_json(bio, INTACT, Method::POST, &url, &params).await?;
        require_ok(INTACT, status)?;
        let body = body.context("IntAct returned an empty interaction search body")?;
        let data = body.get("data").unwrap_or(&body);
        let page_total = data
            .get("totalElements")
            .or(body.get("totalElements"))
            .and_then(Value::as_u64)
            .context("IntAct interaction search lacked totalElements")?;
        if page == 0 {
            total = page_total;
        } else if page_total != total {
            bail!("IntAct totalElements changed mid-page ({total} → {page_total})");
        }
        let content = data
            .get("content")
            .or(body.get("content"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let last = data
            .get("last")
            .or(body.get("last"))
            .and_then(Value::as_bool)
            .unwrap_or(content.is_empty());
        for raw in content {
            if records.len() >= cap {
                break;
            }
            records.push(slim_record(&raw));
        }
        if last || records.len() >= cap || records.len() as u64 >= total {
            break;
        }
        page += 1;
        if page > 20 {
            break;
        }
    }
    sort_records(&mut records);
    Ok(Sweep {
        truncated: total > records.len() as u64,
        total,
        records,
    })
}

async fn resolve_interactors(bio: &NativeBio, query: &str) -> Result<Vec<Value>> {
    let base = api_base(bio, "INTACT_URL", INTACT_DEFAULT);
    let url = format!("{base}/interactor/findInteractor/{}", path_segment(query));
    let mut out = Vec::new();
    let mut page = 0u32;
    loop {
        let (status, body) = send_json(
            bio,
            INTACT,
            Method::GET,
            &url,
            &[
                ("page".into(), page.to_string()),
                ("pageSize".into(), PAGE_SIZE.to_string()),
            ],
        )
        .await?;
        require_ok(INTACT, status)?;
        let body = body.unwrap_or(json!({}));
        let content = body
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let last = body
            .get("last")
            .and_then(Value::as_bool)
            .unwrap_or(content.is_empty());
        for raw in &content {
            out.push(json!({
                "interactor_ac": json_string(raw.get("interactorAc")),
                "preferred_identifier": json!(strip_db(text_field(raw, &["interactorPreferredIdentifier"]))),
                "name": json_string(raw.get("interactorName")),
                "species": json_string(raw.get("interactorSpecies")),
                "taxid": raw.get("interactorTaxId").cloned().unwrap_or(Value::Null),
                "interactor_type": json_string(raw.get("interactorType")),
                "interaction_count": raw.get("interactionCount").cloned().unwrap_or(Value::Null)
            }));
        }
        if last || content.is_empty() || out.len() >= 200 {
            break;
        }
        page += 1;
    }
    Ok(out)
}

async fn fetch_participants(bio: &NativeBio, ac: &str) -> Result<Vec<Value>> {
    let base = api_base(bio, "INTACT_URL", INTACT_DEFAULT);
    let url = format!("{base}/graph/participants/details/{}", path_segment(ac));
    let mut participants = Vec::new();
    let mut page = 0u32;
    loop {
        let (status, body) = send_json(
            bio,
            INTACT,
            Method::GET,
            &url,
            &[
                ("page".into(), page.to_string()),
                ("pageSize".into(), "100".into()),
            ],
        )
        .await?;
        require_ok(INTACT, status)?;
        let body = body.unwrap_or(json!({}));
        let content = body
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let last = body
            .get("last")
            .and_then(Value::as_bool)
            .unwrap_or(content.is_empty());
        for p in content {
            let pid = object_field(&p, "participantId")
                .map(|map| Value::Object(map.clone()))
                .unwrap_or(json!({}));
            participants.push(json!({
                "participant_ac": json_string(p.get("participantAc")),
                "short_label": json_string(p.get("shortLabel")),
                "identifier": json_string(pid.get("identifier")),
                "identifier_database": json_string(object_field(&pid, "database").and_then(|d| d.get("shortName"))),
                "description": json_string(p.get("description")),
                "type": cv_term(p.get("type")),
                "species": json_string(object_field(&p, "species").and_then(|s| s.get("scientificName"))),
                "taxid": object_field(&p, "species").and_then(|s| s.get("taxId")).cloned().unwrap_or(Value::Null),
                "biological_role": cv_term(p.get("biologicalRole")),
                "experimental_role": cv_term(p.get("experimentalRole"))
            }));
        }
        if last || participants.len() >= 200 {
            break;
        }
        page += 1;
    }
    participants.sort_by(|a, b| {
        a.get("participant_ac")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(
                b.get("participant_ac")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            )
    });
    Ok(participants)
}

fn parse_interaction_details(raw: &Value) -> Value {
    let pubn = object_field(raw, "publication").map(|map| Value::Object(map.clone()));
    let xrefs: Vec<Value> = listify(raw.get("xrefs"))
        .into_iter()
        .map(|x| {
            json!({
                "database": json_string(object_field(x, "database").and_then(|d| d.get("shortName"))),
                "database_mi": json_string(object_field(x, "database").and_then(|d| d.get("identifier"))),
                "identifier": json_string(x.get("identifier")),
                "qualifier": json_string(object_field(x, "qualifier").and_then(|d| d.get("shortName")).or(x.get("qualifier")))
            })
        })
        .collect();
    let annotations: Vec<Value> = listify(raw.get("annotations"))
        .into_iter()
        .map(|a| {
            let topic = cv_term(a.get("topic"));
            json!({
                "topic": json_string(topic.get("name")),
                "topic_mi": json_string(topic.get("mi")),
                "description": json_string(a.get("description"))
            })
        })
        .collect();
    json!({
        "interaction_ac": json_string(raw.get("interactionAc")),
        "short_label": json_string(raw.get("shortLabel")),
        "type": cv_term(raw.get("type")),
        "detection_method": cv_term(raw.get("detectionMethod")),
        "host_organism": json_string(raw.get("hostOrganism")),
        "negative": raw.get("negative").cloned().unwrap_or(Value::Null),
        "publication": pubn.map(|p| json!({
            "pubmed_id": json_string(p.get("pubmedId")),
            "title": json_string(p.get("title")),
            "journal": json_string(p.get("journal")),
            "year": p.get("year").cloned().unwrap_or(Value::Null),
            "authors": p.get("authors").cloned().unwrap_or(Value::Null)
        })).unwrap_or(Value::Null),
        "xrefs": xrefs,
        "annotations": annotations,
        "parameters": raw.get("parameters").cloned().unwrap_or_else(|| json!([])),
        "confidences": raw.get("confidences").cloned().unwrap_or_else(|| json!([]))
    })
}

fn slim_record(raw: &Value) -> Value {
    let id_a_raw = text_field(raw, &["idA"]);
    let id_b_raw = text_field(raw, &["idB"]);
    let ac = text_field(raw, &["ac"]).unwrap_or_default();
    json!({
        "interaction_ac": ac,
        "url": interaction_url(&ac),
        "binary_interaction_id": raw.get("binaryInteractionId").cloned().unwrap_or(Value::Null),
        "ac_a": json_string(raw.get("acA")),
        "ac_b": json_string(raw.get("acB")),
        "id_a": json!(strip_db(id_a_raw.clone())),
        "id_b": json!(strip_db(id_b_raw.clone())),
        "id_a_database": json!(db_suffix(id_a_raw)),
        "id_b_database": json!(db_suffix(id_b_raw)),
        "molecule_a": json_string(raw.get("moleculeA")),
        "molecule_b": json_string(raw.get("moleculeB")),
        "species_a": json_string(raw.get("speciesA")),
        "species_b": json_string(raw.get("speciesB")),
        "taxid_a": raw.get("taxIdA").cloned().unwrap_or(Value::Null),
        "taxid_b": raw.get("taxIdB").cloned().unwrap_or(Value::Null),
        "interaction_type": json_string(raw.get("type")),
        "interaction_type_mi": json_string(raw.get("typeMIIdentifier")),
        "detection_method": json_string(raw.get("detectionMethod")),
        "detection_method_mi": json_string(raw.get("detectionMethodMIIdentifier")),
        "experimental_role_a": json_string(raw.get("experimentalRoleA")),
        "experimental_role_b": json_string(raw.get("experimentalRoleB")),
        "host_organism": json_string(raw.get("hostOrganism")),
        "expansion_method": json_string(raw.get("expansionMethod")),
        "mi_score": raw.get("intactMiscore").cloned().unwrap_or(Value::Null),
        "negative": raw.get("negative").cloned().unwrap_or(Value::Null),
        "pubmed_id": json_string(raw.get("publicationPubmedIdentifier")),
        "first_author": json_string(raw.get("firstAuthor")),
        "source_database": json_string(raw.get("sourceDatabase"))
    })
}

fn ingest_edge(edges: &mut BTreeMap<(String, String), Value>, rec: &Value, origin: &str) {
    let ac = rec
        .get("interaction_ac")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let bin = rec
        .get("binary_interaction_id")
        .map(|v| v.to_string())
        .unwrap_or_default();
    edges.entry((ac, bin)).or_insert_with(|| {
        let mut edge = rec.clone();
        edge["origin"] = json!(origin);
        edge
    });
}

fn sort_records(records: &mut [Value]) {
    records.sort_by(|a, b| {
        let sa = a.get("mi_score").and_then(as_f64).unwrap_or(-1.0);
        let sb = b.get("mi_score").and_then(as_f64).unwrap_or(-1.0);
        sb.partial_cmp(&sa)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                a.get("interaction_ac")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .cmp(
                        b.get("interaction_ac")
                            .and_then(Value::as_str)
                            .unwrap_or(""),
                    )
            })
    });
}

fn species_list(raw: Option<&[String]>) -> Result<Vec<String>> {
    let mut out = Vec::new();
    if let Some(items) = raw {
        for item in items {
            let trimmed = item.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.len() > 64 {
                bail!("interactor_species entry exceeds 64 characters");
            }
            out.push(trimmed.to_string());
        }
        if out.len() > 8 {
            bail!("at most 8 interactor_species filters per call");
        }
    }
    Ok(out)
}

fn strip_db(raw: Option<String>) -> Option<String> {
    raw.map(|text| {
        text.split(" (")
            .next()
            .unwrap_or(text.as_str())
            .trim()
            .to_string()
    })
}

fn db_suffix(raw: Option<String>) -> Option<String> {
    raw.and_then(|text| {
        text.split_once(" (")
            .map(|(_, rest)| rest.trim_end_matches(')').to_string())
    })
}

fn cv_term(node: Option<&Value>) -> Value {
    match node {
        Some(Value::Object(map)) => json!({
            "name": json_string(map.get("shortName")),
            "mi": json_string(map.get("identifier"))
        }),
        _ => Value::Null,
    }
}

fn interaction_url(ac: &str) -> String {
    format!("{INTACT_SITE}/details/interaction/{}", path_segment(ac))
}
