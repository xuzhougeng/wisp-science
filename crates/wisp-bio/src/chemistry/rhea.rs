use super::{json_plain, require_range, require_text, rhea_endpoint, send_json, NativeBio, RHEA};
use anyhow::{bail, Context, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const PREFIXES: &str =
    "PREFIX rh: <http://rdf.rhea-db.org/>\nPREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>\n";

pub async fn search_reactions(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Search =
        serde_json::from_value(args.clone()).context("invalid rhea_search_reactions arguments")?;
    let query = require_text(&args.query, "query", 512)?;
    let limit = require_range(args.limit, 1, 500, "limit")?;
    let (query_type, where_clause) = classify_query(query)?;
    let result = run_search(bio, &where_clause, limit).await?;
    Ok(json!({
        "source": "Rhea",
        "query": query,
        "query_type": query_type,
        "api_total": result.total,
        "n_returned": result.reactions.len(),
        "truncated": result.total > result.reactions.len() as u64,
        "reactions": result.reactions
    }))
}

pub async fn get_reaction(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetReaction =
        serde_json::from_value(args.clone()).context("invalid rhea_get_reaction arguments")?;
    let acc = normalize_rhea_id(&args.rhea_id)?;
    let preds = sparql(
        bio,
        &format!("SELECT ?p ?o WHERE {{ ?r rh:accession \"{acc}\" . ?r ?p ?o . }}"),
    )
    .await?;
    if preds.is_empty() {
        bail!("Rhea has no reaction {acc}");
    }
    let mut equation = Value::Null;
    let mut status = Value::Null;
    let mut is_transport = Value::Null;
    let mut is_balanced = Value::Null;
    let mut ec_numbers = Vec::new();
    let mut pubmed_ids = Vec::new();
    let mut directional = Vec::new();
    let mut bidirectional = Value::Null;
    for row in &preds {
        let pred = localname(row.get("p").map(String::as_str).unwrap_or(""));
        let object = row.get("o").cloned().unwrap_or_default();
        match pred.as_str() {
            "equation" => equation = json!(object),
            "status" => status = json!(localname(&object)),
            "isTransport" => is_transport = json!(object == "true"),
            "isChemicallyBalanced" => is_balanced = json!(object == "true"),
            "ec" => ec_numbers.push(Value::String(localname(&object))),
            "citation" => pubmed_ids.push(Value::String(localname(&object))),
            "directionalReaction" => {
                directional.push(Value::String(format!("RHEA:{}", localname(&object))))
            }
            "bidirectionalReaction" => {
                bidirectional = json!(format!("RHEA:{}", localname(&object)))
            }
            _ => {}
        }
    }
    ec_numbers.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    pubmed_ids.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    directional.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    let parts = sparql(
        bio,
        &format!(
            "SELECT ?side ?coefProp ?cacc ?cname WHERE {{\n  ?r rh:accession \"{acc}\" ; rh:side ?side .\n  ?side ?coefProp ?part . ?coefProp rdfs:subPropertyOf rh:contains .\n  ?part rh:compound ?c . ?c rh:accession ?cacc .\n  OPTIONAL {{ ?c rh:name ?cname }}\n}}"
        ),
    )
    .await?;
    let mut left = Vec::new();
    let mut right = Vec::new();
    for row in parts {
        let side = row.get("side").cloned().unwrap_or_default();
        let entry = json!({
            "compound_accession": row.get("cacc").cloned(),
            "name": row.get("cname").cloned(),
            "coefficient": coefficient(row.get("coefProp").map(String::as_str).unwrap_or(""))
        });
        if side.ends_with("_L") {
            left.push(entry);
        } else if side.ends_with("_R") {
            right.push(entry);
        }
    }
    left.sort_by(|a, b| {
        json_plain(&a["compound_accession"]).cmp(&json_plain(&b["compound_accession"]))
    });
    right.sort_by(|a, b| {
        json_plain(&a["compound_accession"]).cmp(&json_plain(&b["compound_accession"]))
    });
    Ok(json!({
        "source": "Rhea",
        "rhea_id": acc,
        "url": format!("https://www.rhea-db.org/rhea/{}", acc.trim_start_matches("RHEA:")),
        "equation": equation,
        "status": status,
        "is_transport": is_transport,
        "is_chemically_balanced": is_balanced,
        "ec_numbers": ec_numbers,
        "pubmed_ids": pubmed_ids,
        "directional_reactions": directional,
        "bidirectional_reaction": bidirectional,
        "left_side": left,
        "right_side": right
    }))
}

struct SearchHits {
    total: u64,
    reactions: Vec<Value>,
}

async fn run_search(bio: &NativeBio, where_clause: &str, limit: usize) -> Result<SearchHits> {
    let rows = sparql(
        bio,
        &format!(
            "SELECT DISTINCT ?accession ?equation ?status WHERE {{\n{where_clause}\n}} ORDER BY ?accession LIMIT {limit}"
        ),
    )
    .await?;
    let count_rows = sparql(
        bio,
        &format!("SELECT (COUNT(DISTINCT ?accession) AS ?n) WHERE {{\n{where_clause}\n}}"),
    )
    .await?;
    let total = count_rows
        .first()
        .and_then(|row| row.get("n"))
        .and_then(|value| value.parse::<u64>().ok())
        .context("Rhea omitted the match count")?;
    let reactions = rows
        .into_iter()
        .map(|row| {
            json!({
                "rhea_id": row.get("accession").cloned(),
                "url": row.get("accession").map(|acc| {
                    format!("https://www.rhea-db.org/rhea/{}", acc.trim_start_matches("RHEA:"))
                }),
                "equation": row.get("equation").cloned(),
                "status": row.get("status").map(|value| localname(value))
            })
        })
        .collect();
    Ok(SearchHits { total, reactions })
}

async fn sparql(bio: &NativeBio, query: &str) -> Result<Vec<BTreeMap<String, String>>> {
    let params = vec![
        ("query".into(), format!("{PREFIXES}{query}")),
        ("format".into(), "application/sparql-results+json".into()),
    ];
    let raw = send_json(bio, RHEA, Method::POST, &rhea_endpoint(bio), &params)
        .await?
        .context("Rhea SPARQL endpoint returned no results document")?;
    let bindings = raw
        .pointer("/results/bindings")
        .and_then(Value::as_array)
        .context("Rhea omitted SPARQL results")?;
    let mut rows = Vec::new();
    for binding in bindings {
        let object = binding
            .as_object()
            .context("Rhea returned an invalid SPARQL row")?;
        let mut row = BTreeMap::new();
        for (var, cell) in object {
            if let Some(value) = cell.get("value").and_then(Value::as_str) {
                row.insert(var.clone(), value.to_string());
            }
        }
        rows.push(row);
    }
    Ok(rows)
}

fn classify_query(query: &str) -> Result<(&'static str, String)> {
    if is_chebi_query(query) {
        let id = super::chebi::normalize_chebi_id(query)?;
        let uri = format!("<http://purl.obolibrary.org/obo/CHEBI_{id}>");
        let where_clause = format!(
            "  ?r rdfs:subClassOf rh:Reaction ; rh:accession ?accession ;\n     rh:equation ?equation ; rh:status ?status ;\n     rh:side/rh:contains/rh:compound ?c .\n  {{ ?c rh:chebi {uri} }}\n  UNION {{ ?c rh:reactivePart/rh:chebi {uri} }}\n  UNION {{ ?c rh:underlyingChebi {uri} }}"
        );
        return Ok(("chebi", where_clause));
    }
    if is_full_ec(query) {
        let where_clause = format!(
            "  ?r rdfs:subClassOf rh:Reaction ; rh:accession ?accession ;\n     rh:equation ?equation ; rh:status ?status ;\n     rh:ec <http://purl.uniprot.org/enzyme/{query}> ."
        );
        return Ok(("ec", where_clause));
    }
    if is_partial_ec(query) {
        bail!(
            "Rhea search requires a complete EC number (for example 2.1.1.160); partial classes such as 2.1.1.- are not searched"
        );
    }
    let needle = sparql_escape(&query.to_ascii_lowercase());
    let where_clause = format!(
        "  ?r rdfs:subClassOf rh:Reaction ; rh:accession ?accession ;\n     rh:equation ?equation ; rh:status ?status .\n  FILTER(CONTAINS(LCASE(STR(?equation)), \"{needle}\"))"
    );
    Ok(("text", where_clause))
}

fn is_chebi_query(query: &str) -> bool {
    let digits = query
        .strip_prefix("CHEBI:")
        .or_else(|| query.strip_prefix("chebi:"))
        .unwrap_or(query);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

fn is_full_ec(query: &str) -> bool {
    let parts: Vec<_> = query.split('.').collect();
    parts.len() == 4
        && parts[0].bytes().all(|b| b.is_ascii_digit())
        && !parts[0].is_empty()
        && parts[1].bytes().all(|b| b.is_ascii_digit())
        && !parts[1].is_empty()
        && parts[2].bytes().all(|b| b.is_ascii_digit())
        && !parts[2].is_empty()
        && {
            let last = parts[3].strip_prefix('n').unwrap_or(parts[3]);
            !last.is_empty() && last.bytes().all(|b| b.is_ascii_digit())
        }
}

fn is_partial_ec(query: &str) -> bool {
    let parts: Vec<_> = query.split('.').collect();
    if !(2..=4).contains(&parts.len()) {
        return false;
    }
    parts
        .iter()
        .all(|part| *part == "-" || part.bytes().all(|b| b.is_ascii_digit()))
        && parts.iter().any(|part| *part == "-" || parts.len() < 4)
}

pub(super) fn normalize_rhea_id(value: &str) -> Result<String> {
    let value = require_text(value, "rhea_id", 32)?;
    let digits = value
        .strip_prefix("RHEA:")
        .or_else(|| value.strip_prefix("rhea:"))
        .unwrap_or(value);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        bail!("rhea_id must be RHEA:<integer> or a bare integer");
    }
    let id: u64 = digits
        .parse()
        .ok()
        .filter(|id| *id > 0)
        .context("rhea_id must be RHEA:<integer> or a bare integer")?;
    Ok(format!("RHEA:{id}"))
}

fn sparql_escape(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => {}
            c => out.push(c),
        }
    }
    out
}

fn localname(uri: &str) -> String {
    uri.trim_end_matches('/')
        .rsplit(['/', '#'])
        .next()
        .unwrap_or(uri)
        .to_string()
}

fn coefficient(uri: &str) -> String {
    let local = localname(uri);
    let rest = local.strip_prefix("contains").unwrap_or("");
    if rest.is_empty() {
        "1".into()
    } else {
        rest.to_string()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    50
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetReaction {
    rhea_id: String,
}
