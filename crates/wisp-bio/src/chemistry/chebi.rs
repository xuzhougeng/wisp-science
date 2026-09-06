use super::{
    as_object_array, cap, chebi_base, join_url, json_opt, json_plain, json_u64, require_range,
    require_text, send_json, NativeBio, EBI,
};
use anyhow::{bail, Context, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Map, Value};

pub async fn search(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Search =
        serde_json::from_value(args.clone()).context("invalid chebi_search arguments")?;
    let term = require_text(&args.term, "term", 512)?;
    let size = require_range(args.max_results, 1, 100, "max_results")?;
    let page = require_range(args.page, 1, 10_000, "page")?;
    let params = vec![
        ("term".into(), term.to_string()),
        ("size".into(), size.to_string()),
        ("page".into(), page.to_string()),
    ];
    let url = join_url(&chebi_base(bio), "es_search/");
    let raw = send_json(bio, EBI, Method::GET, &url, &params).await?;
    let Some(raw) = raw else {
        return Ok(json!({
            "source": "ChEBI",
            "term": term,
            "page": page,
            "size": size,
            "api_total": 0,
            "number_pages": 0,
            "has_more": false,
            "results": []
        }));
    };
    let api_total = elastic_total(&raw).context("ChEBI omitted the search total")?;
    let number_pages = json_u64(&raw["number_pages"]).unwrap_or_else(|| {
        if size == 0 {
            0
        } else {
            api_total.div_ceil(size as u64)
        }
    });
    let mut results = Vec::new();
    for hit in raw
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let src = hit.get("_source").unwrap_or(hit);
        let accession = chebi_accession(src).or_else(|| chebi_accession(hit));
        let mut row = Map::new();
        row.insert(
            "chebi_accession".into(),
            accession.clone().map(Value::String).unwrap_or(Value::Null),
        );
        row.insert("name".into(), json_opt(src, "name"));
        row.insert("definition".into(), json_opt(src, "definition"));
        row.insert("stars".into(), json_opt(src, "stars"));
        row.insert("formula".into(), json_opt(src, "formula"));
        row.insert("charge".into(), json_opt(src, "charge"));
        row.insert("mass".into(), json_opt(src, "mass"));
        row.insert(
            "monoisotopic_mass".into(),
            src.get("monoisotopic_mass")
                .or_else(|| src.get("monoisotopicmass"))
                .cloned()
                .unwrap_or(Value::Null),
        );
        row.insert("smiles".into(), json_opt(src, "smiles"));
        row.insert("inchikey".into(), json_opt(src, "inchikey"));
        row.insert("score".into(), json_opt(hit, "_score"));
        if let Some(acc) = accession {
            row.insert("url".into(), json!(entity_url(&acc)));
        }
        results.push(Value::Object(row));
    }
    Ok(json!({
        "source": "ChEBI",
        "term": term,
        "page": page,
        "size": size,
        "api_total": api_total,
        "number_pages": number_pages,
        "has_more": api_total > (page as u64) * (size as u64),
        "results": results
    }))
}

pub async fn get_entity(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetEntity =
        serde_json::from_value(args.clone()).context("invalid chebi_get_entity arguments")?;
    let max_synonyms = require_range(args.max_synonyms, 1, 200, "max_synonyms")?;
    let max_xrefs = require_range(args.max_xrefs, 1, 200, "max_xrefs")?;
    let mut record = fetch_compound(bio, &args.chebi_id).await?;
    let synonyms = record
        .as_object_mut()
        .and_then(|object| object.remove("synonyms"))
        .unwrap_or_else(|| json!([]));
    let xrefs = record
        .as_object_mut()
        .and_then(|object| object.remove("xrefs"))
        .unwrap_or_else(|| json!([]));
    if let Some(object) = record.as_object_mut() {
        object.remove("outgoing_relations");
        object.remove("incoming_relations");
    }
    let synonym_list = synonyms.as_array().cloned().unwrap_or_default();
    let xref_list = xrefs.as_array().cloned().unwrap_or_default();
    let (syn_page, syn_trunc) = cap(&synonym_list, max_synonyms);
    let (xref_page, xref_trunc) = cap(&xref_list, max_xrefs);
    if let Some(object) = record.as_object_mut() {
        object.insert("synonyms".into(), json!(syn_page));
        object.insert("n_synonyms_total".into(), json!(synonym_list.len()));
        object.insert("synonyms_truncated".into(), json!(syn_trunc));
        object.insert("xrefs".into(), json!(xref_page));
        object.insert("n_xrefs_total".into(), json!(xref_list.len()));
        object.insert("xrefs_truncated".into(), json!(xref_trunc));
    }
    Ok(record)
}

pub async fn get_ontology(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetOntology =
        serde_json::from_value(args.clone()).context("invalid chebi_get_ontology arguments")?;
    let max_relations = require_range(args.max_relations, 1, 1000, "max_relations")?;
    let record = fetch_compound(bio, &args.chebi_id).await?;
    let filter = args
        .relation_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let outgoing = filter_relations(&record["outgoing_relations"], filter);
    let incoming = filter_relations(&record["incoming_relations"], filter);
    let (out_page, out_trunc) = cap(&outgoing, max_relations);
    let (in_page, in_trunc) = cap(&incoming, max_relations);
    Ok(json!({
        "source": "ChEBI",
        "chebi_accession": record["chebi_accession"],
        "name": record["name"],
        "url": record["url"],
        "relation_type_filter": filter,
        "outgoing_relations": out_page,
        "n_outgoing_total": outgoing.len(),
        "outgoing_truncated": out_trunc,
        "incoming_relations": in_page,
        "n_incoming_total": incoming.len(),
        "incoming_truncated": in_trunc
    }))
}

async fn fetch_compound(bio: &NativeBio, chebi_id: &str) -> Result<Value> {
    let id = normalize_chebi_id(chebi_id)?;
    let url = join_url(&chebi_base(bio), &format!("compound/{id}/"));
    let raw = send_json(bio, EBI, Method::GET, &url, &[])
        .await?
        .with_context(|| format!("ChEBI has no entity CHEBI:{id}"))?;
    Ok(normalize_compound(&raw, id))
}

pub(super) fn normalize_chebi_id(value: &str) -> Result<u64> {
    let value = require_text(value, "chebi_id", 32)?;
    let digits = value
        .strip_prefix("CHEBI:")
        .or_else(|| value.strip_prefix("chebi:"))
        .unwrap_or(value);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        bail!("chebi_id must be CHEBI:<integer> or a bare integer");
    }
    digits
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
        .context("chebi_id must be CHEBI:<integer> or a bare integer")
}

fn normalize_compound(raw: &Value, requested: u64) -> Value {
    let names = raw.get("names");
    let mut synonyms = Vec::new();
    let mut iupac_names = Vec::new();
    if let Some(Value::Object(groups)) = names {
        for (kind, entries) in groups {
            let label = kind.to_ascii_uppercase();
            for entry in as_object_array(Some(entries)) {
                if let Some(name) = json_plain(&entry["name"]) {
                    if label.contains("IUPAC") {
                        iupac_names.push(Value::String(name));
                    } else if label.contains("SYNONYM") {
                        synonyms.push(Value::String(name));
                    }
                }
            }
        }
    }
    let chem = raw.get("chemical_data").unwrap_or(&Value::Null);
    let structure = raw.get("default_structure").unwrap_or(&Value::Null);
    let mut xrefs = Vec::new();
    if let Some(Value::Object(groups)) = raw.get("database_accessions") {
        let mut keys: Vec<_> = groups.keys().cloned().collect();
        keys.sort();
        for key in keys {
            for entry in as_object_array(groups.get(&key)) {
                xrefs.push(json!({
                    "type": key,
                    "accession": json_opt(entry, "accession_number"),
                    "source": json_opt(entry, "source_name"),
                    "url": json_opt(entry, "url")
                }));
            }
        }
    }
    let relations = raw.get("ontology_relations").unwrap_or(&Value::Null);
    let outgoing = as_object_array(relations.get("outgoing_relations"))
        .into_iter()
        .filter_map(normalize_relation)
        .collect::<Vec<_>>();
    let incoming = as_object_array(relations.get("incoming_relations"))
        .into_iter()
        .filter_map(normalize_relation)
        .collect::<Vec<_>>();
    let roles = as_object_array(raw.get("roles_classification"))
        .into_iter()
        .map(|role| {
            json!({
                "chebi_accession": chebi_accession(role).map(Value::String).unwrap_or(Value::Null),
                "name": json_opt(role, "name"),
                "definition": json_opt(role, "definition")
            })
        })
        .collect::<Vec<_>>();
    let accession = chebi_accession(raw).unwrap_or_else(|| format!("CHEBI:{requested}"));
    let secondary = match raw.get("secondary_ids") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| json_plain(item).or_else(|| chebi_accession(item)))
            .map(Value::String)
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    json!({
        "source": "ChEBI",
        "chebi_accession": accession,
        "url": entity_url(&accession),
        "name": json_opt(raw, "name"),
        "definition": json_opt(raw, "definition"),
        "stars": json_opt(raw, "stars"),
        "formula": json_opt(chem, "formula"),
        "charge": json_opt(chem, "charge"),
        "mass": json_opt(chem, "mass"),
        "monoisotopic_mass": json_opt(chem, "monoisotopic_mass"),
        "smiles": json_opt(structure, "smiles"),
        "inchi": structure.get("standard_inchi").cloned().unwrap_or(Value::Null),
        "inchikey": structure.get("standard_inchi_key").cloned().unwrap_or(Value::Null),
        "iupac_names": iupac_names,
        "synonyms": synonyms,
        "secondary_ids": secondary,
        "xrefs": xrefs,
        "outgoing_relations": outgoing,
        "incoming_relations": incoming,
        "roles": roles,
        "modified_on": json_opt(raw, "modified_on"),
        "is_released": json_opt(raw, "is_released")
    })
}

fn normalize_relation(rel: &Value) -> Option<Value> {
    Some(json!({
        "relation_type": json_plain(&rel["relation_type"])?,
        "init_chebi_id": chebi_acc_field(rel, "init_id").or_else(|| chebi_acc_field(rel, "init_chebi_id")),
        "init_name": json_opt(rel, "init_name"),
        "final_chebi_id": chebi_acc_field(rel, "final_id").or_else(|| chebi_acc_field(rel, "final_chebi_id")),
        "final_name": json_opt(rel, "final_name")
    }))
}

fn filter_relations(value: &Value, relation_type: Option<&str>) -> Vec<Value> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter(|rel| {
            relation_type.is_none_or(|wanted| {
                rel.get("relation_type").and_then(Value::as_str) == Some(wanted)
            })
        })
        .cloned()
        .collect()
}

fn chebi_accession(value: &Value) -> Option<String> {
    if let Some(text) = json_plain(&value["chebi_accession"]) {
        return Some(canonical_chebi(&text));
    }
    if let Some(id) = json_u64(&value["id"]) {
        return Some(format!("CHEBI:{id}"));
    }
    None
}

fn chebi_acc_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).map(|item| {
        if let Some(text) = json_plain(item) {
            canonical_chebi(&text)
        } else if let Some(id) = json_u64(item) {
            format!("CHEBI:{id}")
        } else {
            item.to_string()
        }
    })
}

fn canonical_chebi(value: &str) -> String {
    let value = value.trim();
    if let Some(digits) = value
        .strip_prefix("CHEBI:")
        .or_else(|| value.strip_prefix("chebi:"))
    {
        format!("CHEBI:{digits}")
    } else if value.bytes().all(|b| b.is_ascii_digit()) {
        format!("CHEBI:{value}")
    } else {
        value.to_string()
    }
}

fn entity_url(accession: &str) -> String {
    format!("https://www.ebi.ac.uk/chebi/searchId.do?chebiId={accession}")
}

fn elastic_total(raw: &Value) -> Option<u64> {
    match raw.get("total") {
        Some(Value::Number(number)) => number.as_u64(),
        Some(Value::Object(object)) => object.get("value").and_then(json_u64),
        Some(Value::String(text)) => text.parse().ok(),
        _ => json_u64(&raw["api_total"]),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    term: String,
    #[serde(default = "default_size")]
    max_results: usize,
    #[serde(default = "default_page")]
    page: usize,
}

fn default_size() -> usize {
    20
}
fn default_page() -> usize {
    1
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetEntity {
    chebi_id: String,
    #[serde(default = "default_max_synonyms")]
    max_synonyms: usize,
    #[serde(default = "default_max_xrefs")]
    max_xrefs: usize,
}

fn default_max_synonyms() -> usize {
    30
}
fn default_max_xrefs() -> usize {
    50
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetOntology {
    chebi_id: String,
    relation_type: Option<String>,
    #[serde(default = "default_max_relations")]
    max_relations: usize,
}

fn default_max_relations() -> usize {
    100
}
