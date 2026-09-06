use super::{
    bound_page, default_architecture_max, default_clan_max, default_max, default_true,
    display_name, get_json, interpro_base, json_u64, metadata, page_size_for, path_segment,
    require_ids, require_text, resolve_next, taxon_id, text_field, Fetch, INTERPRO, INTERPRO_SITE,
    MAX_PAGES, MAX_PROTEINS,
};
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

const ENTRY_TYPES: &[&str] = &[
    "family",
    "domain",
    "repeat",
    "homologous_superfamily",
    "conserved_site",
    "active_site",
    "binding_site",
    "ptm",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DomainArgs {
    pub accessions: Vec<String>,
    #[serde(default = "default_architecture_max")]
    pub max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryArgs {
    accession: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClanArgs {
    clan_accession: String,
    #[serde(default = "default_clan_max")]
    max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FamilyProteins {
    pfam_accession: String,
    #[serde(default)]
    reviewed_only: bool,
    tax_id: Option<i64>,
    #[serde(default)]
    count_only: bool,
    #[serde(default = "default_max")]
    max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FamilyProteomes {
    pfam_accession: String,
    #[serde(default = "default_true")]
    count_only: bool,
    #[serde(default = "default_max")]
    max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchEntries {
    pub query: Option<String>,
    pub entry_type: Option<String>,
    #[serde(default = "default_interpro_db")]
    pub source_db: String,
    pub go_term: Option<String>,
    #[serde(default = "default_max")]
    pub max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchClans {
    query: Option<String>,
    #[serde(default = "default_max")]
    max_results: u32,
}

fn default_interpro_db() -> String {
    "interpro".into()
}

pub(super) async fn get_domain_architecture(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: DomainArgs =
        serde_json::from_value(args.clone()).context("invalid domain architecture arguments")?;
    let accessions = require_ids(&args.accessions, MAX_PROTEINS, "UniProt accession")?;
    for acc in &accessions {
        if !is_uniprot(acc) {
            bail!("{acc:?} is not a UniProt accession");
        }
    }
    let cap = bound_page(args.max_results)?;
    let mut proteins = Vec::new();
    let mut missing = Vec::new();
    for acc in &accessions {
        match protein_entries(bio, acc, cap).await? {
            Some(protein) => proteins.push(protein),
            None => missing.push(acc.clone()),
        }
    }
    Ok(json!({
        "source": "InterPro",
        "source_url": format!("{INTERPRO_SITE}/"),
        "query": {"accessions": accessions, "max_results": cap},
        "returned": proteins.len(),
        "missing_ids": missing,
        "proteins": proteins
    }))
}

pub(super) async fn get_interpro_entry(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: EntryArgs =
        serde_json::from_value(args.clone()).context("invalid InterPro entry arguments")?;
    let accession = require_text(&args.accession, "accession", 16)?.to_ascii_uppercase();
    let db = entry_db(&accession)?;
    let url = format!(
        "{}/entry/{db}/{}/",
        interpro_base(bio),
        path_segment(&accession)
    );
    match get_json(bio, INTERPRO, &url, &[]).await? {
        Fetch::Json(payload) => Ok(project_entry_detail(&payload, &accession, db)),
        Fetch::Empty | Fetch::NotFound => {
            bail!("InterPro accession {accession} was not found")
        }
    }
}

pub(super) async fn get_pfam_clan(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ClanArgs =
        serde_json::from_value(args.clone()).context("invalid Pfam clan arguments")?;
    let accession = require_text(&args.clan_accession, "clan accession", 16)?.to_ascii_uppercase();
    if !is_clan(&accession) {
        bail!("{accession:?} is not a Pfam clan accession (CLxxxx)");
    }
    let cap = bound_page(args.max_results)?;
    let url = format!(
        "{}/set/pfam/{}/",
        interpro_base(bio),
        path_segment(&accession)
    );
    let payload = match get_json(bio, INTERPRO, &url, &[]).await? {
        Fetch::Json(payload) => payload,
        Fetch::Empty | Fetch::NotFound => bail!("Pfam clan {accession} was not found"),
    };
    Ok(project_clan(&payload, cap))
}

pub(super) async fn get_pfam_family_proteins(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: FamilyProteins =
        serde_json::from_value(args.clone()).context("invalid Pfam family protein arguments")?;
    let accession = pfam_accession(&args.pfam_accession)?;
    let cap = bound_page(args.max_results)?;
    let db = if args.reviewed_only {
        "reviewed"
    } else {
        "uniprot"
    };
    let mut params = Vec::new();
    if let Some(tax) = args.tax_id {
        params.push(("tax_id".into(), taxon_id(tax, "tax_id")?.to_string()));
    }
    let path = format!(
        "{}/protein/{db}/entry/pfam/{}/",
        interpro_base(bio),
        path_segment(&accession)
    );
    let page = list_page(bio, &path, params, cap, args.count_only).await?;
    Ok(json!({
        "source": "InterPro",
        "source_url": entry_url("pfam", &accession),
        "query": {
            "pfam_accession": accession,
            "reviewed_only": args.reviewed_only,
            "tax_id": args.tax_id,
            "count_only": args.count_only,
            "max_results": cap
        },
        "total": page.total,
        "returned": page.records.len(),
        "has_more": page.has_more,
        "count_only": args.count_only,
        "results": page.records.into_iter().map(project_protein).collect::<Vec<_>>()
    }))
}

pub(super) async fn get_pfam_family_proteomes(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: FamilyProteomes =
        serde_json::from_value(args.clone()).context("invalid Pfam family proteome arguments")?;
    let accession = pfam_accession(&args.pfam_accession)?;
    let cap = bound_page(args.max_results)?;
    let path = format!(
        "{}/proteome/uniprot/entry/pfam/{}/",
        interpro_base(bio),
        path_segment(&accession)
    );
    let page = list_page(bio, &path, Vec::new(), cap, args.count_only).await?;
    Ok(json!({
        "source": "InterPro",
        "source_url": entry_url("pfam", &accession),
        "query": {
            "pfam_accession": accession,
            "count_only": args.count_only,
            "max_results": cap
        },
        "total": page.total,
        "returned": page.records.len(),
        "has_more": page.has_more,
        "count_only": args.count_only,
        "results": page.records.into_iter().map(project_proteome).collect::<Vec<_>>()
    }))
}

pub(super) async fn search_interpro_entries(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchEntries =
        serde_json::from_value(args.clone()).context("invalid InterPro search arguments")?;
    let query = args
        .query
        .as_deref()
        .map(|value| require_text(value, "query", super::MAX_QUERY))
        .transpose()?;
    let go_term = args
        .go_term
        .as_deref()
        .map(|value| require_text(value, "go_term", 16))
        .transpose()?;
    if query.is_none() && go_term.is_none() {
        bail!("provide query and/or go_term; an unfiltered InterPro dump is not returned");
    }
    if let Some(term) = go_term.as_deref() {
        if !is_go_term(term) {
            bail!("{term:?} is not a GO identifier (GO:#######)");
        }
    }
    let source_db = source_db(&args.source_db)?;
    let entry_type = args
        .entry_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(kind) = entry_type {
        if !ENTRY_TYPES.contains(&kind) {
            bail!("{kind:?} is not an InterPro entry type");
        }
    }
    let cap = bound_page(args.max_results)?;
    let mut params = Vec::new();
    if let Some(query) = &query {
        params.push(("search".into(), query.clone()));
    }
    if let Some(kind) = entry_type {
        params.push(("type".into(), kind.to_string()));
    }
    if let Some(term) = &go_term {
        params.push(("go_term".into(), term.clone()));
    }
    let path = format!("{}/entry/{source_db}/", interpro_base(bio));
    let page = list_page(bio, &path, params, cap, false).await?;
    Ok(json!({
        "source": "InterPro",
        "source_url": format!("{INTERPRO_SITE}/entry/{source_db}/"),
        "query": {
            "query": query,
            "entry_type": entry_type,
            "source_db": source_db,
            "go_term": go_term,
            "max_results": cap
        },
        "total": page.total,
        "returned": page.records.len(),
        "has_more": page.has_more,
        "results": page.records.into_iter().map(|row| project_entry_row(&row)).collect::<Vec<_>>()
    }))
}

pub(super) async fn search_pfam_clans(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchClans =
        serde_json::from_value(args.clone()).context("invalid Pfam clan search arguments")?;
    let query = args
        .query
        .as_deref()
        .map(|value| require_text(value, "query", super::MAX_QUERY))
        .transpose()?;
    let cap = bound_page(args.max_results)?;
    let mut params = Vec::new();
    if let Some(query) = &query {
        params.push(("search".into(), query.clone()));
    }
    let path = format!("{}/set/pfam/", interpro_base(bio));
    let page = list_page(bio, &path, params, cap, false).await?;
    Ok(json!({
        "source": "InterPro",
        "source_url": format!("{INTERPRO_SITE}/set/pfam/"),
        "query": {"query": query, "max_results": cap},
        "total": page.total,
        "returned": page.records.len(),
        "has_more": page.has_more,
        "results": page.records.into_iter().map(project_clan_row).collect::<Vec<_>>()
    }))
}

struct Page {
    total: u64,
    has_more: bool,
    records: Vec<Value>,
}

async fn protein_entries(bio: &NativeBio, accession: &str, cap: usize) -> Result<Option<Value>> {
    let path = format!(
        "{}/entry/interpro/protein/uniprot/{}/",
        interpro_base(bio),
        path_segment(&accession.to_ascii_uppercase())
    );
    let page = match list_page(bio, &path, Vec::new(), cap, false).await {
        Ok(page) => page,
        Err(error) if error.to_string().contains("HTTP 404") => return Ok(None),
        Err(error) => return Err(error),
    };
    let protein_length = page.records.iter().find_map(|row| {
        protein_match(row, accession)
            .and_then(|protein| json_u64(protein.get("protein_length")).ok())
    });
    let entries: Vec<Value> = page
        .records
        .iter()
        .map(|row| project_architecture_entry(row, accession))
        .collect();
    Ok(Some(json!({
        "accession": accession.to_ascii_uppercase(),
        "protein_length": protein_length,
        "url": format!("{INTERPRO_SITE}/protein/UniProt/{}/", path_segment(&accession.to_ascii_uppercase())),
        "total_entries": page.total,
        "returned": entries.len(),
        "has_more": page.has_more,
        "entries": entries
    })))
}

async fn list_page(
    bio: &NativeBio,
    path: &str,
    mut params: Vec<(String, String)>,
    cap: usize,
    count_only: bool,
) -> Result<Page> {
    let size = if count_only { 1 } else { page_size_for(cap) };
    params.push(("page_size".into(), size.to_string()));
    let mut url = path.to_string();
    let mut query = params;
    let mut records = Vec::new();
    let mut total = 0u64;
    let mut next_link = None;
    for page in 0..MAX_PAGES {
        let fetched = get_json(bio, INTERPRO, &url, &query).await?;
        query = Vec::new();
        match fetched {
            Fetch::Empty if page == 0 => {
                return Ok(Page {
                    total: 0,
                    has_more: false,
                    records: Vec::new(),
                });
            }
            Fetch::Empty => break,
            Fetch::NotFound => bail!("InterPro returned HTTP 404"),
            Fetch::Json(payload) => {
                total = json_u64(payload.get("count"))?;
                if count_only {
                    return Ok(Page {
                        total,
                        has_more: total > 0,
                        records: Vec::new(),
                    });
                }
                let Some(Value::Array(rows)) = payload.get("results") else {
                    bail!("InterPro listing omitted results");
                };
                for row in rows {
                    if records.len() >= cap {
                        break;
                    }
                    records.push(row.clone());
                }
                next_link = resolve_next(
                    &interpro_base(bio),
                    payload.get("next").unwrap_or(&Value::Null),
                )?;
                if records.len() >= cap || next_link.is_none() {
                    break;
                }
                url = next_link.clone().unwrap();
            }
        }
    }
    let has_more = (total as usize) > records.len() || next_link.is_some();
    Ok(Page {
        total,
        has_more,
        records,
    })
}

fn project_entry_row(row: &Value) -> Value {
    let md = metadata(row);
    let accession = text_field(md, "accession");
    let db = text_field(md, "source_database").unwrap_or_else(|| "interpro".into());
    json!({
        "accession": accession,
        "name": display_name(md.get("name")),
        "type": text_field(md, "type"),
        "source_database": db,
        "integrated": md.get("integrated").cloned().unwrap_or(Value::Null),
        "member_db_signatures": member_signatures(md.get("member_databases")),
        "go_terms": go_terms(md.get("go_terms")),
        "url": accession.as_deref().map(|acc| entry_url(&db, acc))
    })
}

fn project_entry_detail(payload: &Value, accession: &str, db: &str) -> Value {
    let md = metadata(payload);
    let name = match md.get("name") {
        Some(Value::Object(map)) => json!({
            "name": map.get("name"),
            "short": map.get("short")
        }),
        other => json!({"name": display_name(other), "short": Value::Null}),
    };
    json!({
        "source": "InterPro",
        "source_url": entry_url(db, accession),
        "accession": text_field(md, "accession").unwrap_or_else(|| accession.to_string()),
        "name": name,
        "type": text_field(md, "type"),
        "source_database": text_field(md, "source_database").unwrap_or_else(|| db.to_string()),
        "integrated": md.get("integrated").cloned().unwrap_or(Value::Null),
        "set_info": md.get("set_info").cloned().unwrap_or(Value::Null),
        "member_db_signatures": member_signatures(md.get("member_databases")),
        "go_terms": go_terms(md.get("go_terms")),
        "description": plain_description(md.get("description"))
    })
}

fn project_clan(payload: &Value, cap: usize) -> Value {
    let md = metadata(payload);
    let accession = text_field(md, "accession");
    let mut members: Vec<Value> = md
        .get("relationships")
        .and_then(|rel| rel.get("nodes"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|node| {
            json!({
                "accession": text_field(&node, "accession"),
                "name": display_name(node.get("name")).or_else(|| text_field(&node, "name")),
                "short_name": text_field(&node, "short_name"),
                "type": text_field(&node, "type"),
                "url": text_field(&node, "accession").map(|acc| entry_url("pfam", &acc))
            })
        })
        .collect();
    members.sort_by(|a, b| {
        a.get("accession")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(b.get("accession").and_then(Value::as_str).unwrap_or(""))
    });
    let member_count = members.len();
    let truncated = members.len() > cap;
    members.truncate(cap);
    json!({
        "source": "InterPro",
        "source_url": accession.as_deref().map(|acc| format!("{INTERPRO_SITE}/set/pfam/{acc}/")),
        "accession": accession,
        "name": display_name(md.get("name")),
        "source_database": text_field(md, "source_database"),
        "member_count": member_count,
        "returned": members.len(),
        "has_more": truncated,
        "members": members
    })
}

fn project_clan_row(row: Value) -> Value {
    let md = metadata(&row);
    let accession = text_field(md, "accession");
    json!({
        "accession": accession,
        "name": display_name(md.get("name")),
        "source_database": text_field(md, "source_database"),
        "url": accession.map(|acc| format!("{INTERPRO_SITE}/set/pfam/{acc}/"))
    })
}

fn project_protein(row: Value) -> Value {
    let md = metadata(&row);
    let organism = md.get("source_organism").cloned().unwrap_or(Value::Null);
    json!({
        "accession": text_field(md, "accession"),
        "name": display_name(md.get("name")).or_else(|| text_field(md, "name")),
        "source_database": text_field(md, "source_database"),
        "length": md.get("length").cloned().unwrap_or(Value::Null),
        "tax_id": organism.get("taxId").cloned().unwrap_or(Value::Null),
        "organism": organism.get("scientificName").cloned().unwrap_or(Value::Null),
        "url": text_field(md, "accession").map(|acc| format!("{INTERPRO_SITE}/protein/UniProt/{acc}/"))
    })
}

fn project_proteome(row: Value) -> Value {
    let md = metadata(&row);
    json!({
        "accession": text_field(md, "accession"),
        "name": display_name(md.get("name")).or_else(|| text_field(md, "name")),
        "is_reference": md.get("is_reference").cloned().unwrap_or(Value::Null),
        "taxonomy": md.get("taxonomy").cloned().unwrap_or(Value::Null)
    })
}

fn project_architecture_entry(row: &Value, protein: &str) -> Value {
    let md = metadata(row);
    let accession = text_field(md, "accession");
    json!({
        "accession": accession,
        "name": display_name(md.get("name")),
        "type": text_field(md, "type"),
        "source_database": text_field(md, "source_database"),
        "member_db_signatures": member_signatures(md.get("member_databases")),
        "go_terms": go_terms(md.get("go_terms")),
        "locations": locations(protein_match(row, protein)),
        "url": accession.map(|acc| entry_url("interpro", &acc))
    })
}

fn protein_match<'a>(row: &'a Value, accession: &str) -> Option<&'a Value> {
    let proteins = row.get("proteins")?.as_array()?;
    proteins
        .iter()
        .find(|protein| {
            protein
                .get("accession")
                .and_then(Value::as_str)
                .is_some_and(|value| value.eq_ignore_ascii_case(accession))
        })
        .or_else(|| proteins.first())
}

fn locations(protein: Option<&Value>) -> Value {
    let Some(protein) = protein else {
        return json!([]);
    };
    let Some(Value::Array(locs)) = protein.get("entry_protein_locations") else {
        return json!([]);
    };
    Value::Array(
        locs.iter()
            .map(|loc| {
                let fragments: Vec<Value> = loc
                    .get("fragments")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|frag| {
                        let mut out = json!({
                            "start": frag.get("start"),
                            "end": frag.get("end")
                        });
                        if let Some(status) = frag.get("dc-status").and_then(Value::as_str) {
                            if status != "CONTINUOUS" {
                                out["dc_status"] = json!(status);
                            }
                        }
                        out
                    })
                    .collect();
                let mut out = json!({"fragments": fragments});
                if loc.get("representative") == Some(&Value::Bool(true)) {
                    out["representative"] = json!(true);
                }
                if loc.get("model").is_some_and(|value| !value.is_null()) {
                    out["model"] = loc["model"].clone();
                }
                if loc.get("score").is_some_and(|value| !value.is_null()) {
                    out["score"] = loc["score"].clone();
                }
                out
            })
            .collect(),
    )
}

fn member_signatures(value: Option<&Value>) -> Value {
    let Some(Value::Object(map)) = value else {
        return json!([]);
    };
    let mut out = Vec::new();
    for (db, sigs) in map {
        if let Value::Object(sigs) = sigs {
            for (accession, name) in sigs {
                out.push(json!({
                    "database": db,
                    "accession": accession,
                    "name": name
                }));
            }
        }
    }
    out.sort_by(|a, b| {
        let db = a["database"].as_str().unwrap_or("");
        let acc = a["accession"].as_str().unwrap_or("");
        let other = (
            b["database"].as_str().unwrap_or(""),
            b["accession"].as_str().unwrap_or(""),
        );
        (db, acc).cmp(&other)
    });
    Value::Array(out)
}

fn go_terms(value: Option<&Value>) -> Value {
    let Some(Value::Array(terms)) = value else {
        return json!([]);
    };
    let mut out: Vec<Value> = terms
        .iter()
        .map(|term| {
            json!({
                "identifier": text_field(term, "identifier"),
                "name": text_field(term, "name"),
                "category": term.get("category").and_then(|cat| cat.get("code")).cloned().or_else(|| term.get("category").cloned())
            })
        })
        .collect();
    out.sort_by(|a, b| {
        a["identifier"]
            .as_str()
            .unwrap_or("")
            .cmp(b["identifier"].as_str().unwrap_or(""))
    });
    Value::Array(out)
}

fn plain_description(value: Option<&Value>) -> Value {
    let Some(value) = value else {
        return Value::Null;
    };
    match value {
        Value::String(text) if !text.contains('<') && text.len() <= 2000 => json!(text),
        Value::Array(parts) => {
            let joined: Vec<&str> = parts
                .iter()
                .filter_map(|part| {
                    part.get("text")
                        .and_then(Value::as_str)
                        .or_else(|| part.as_str())
                })
                .collect();
            if joined.is_empty() {
                Value::Null
            } else {
                json!(joined.join(" "))
            }
        }
        _ => Value::Null,
    }
}

fn entry_url(db: &str, accession: &str) -> String {
    let folder = if db.eq_ignore_ascii_case("interpro") {
        "InterPro"
    } else {
        db
    };
    format!("{INTERPRO_SITE}/entry/{folder}/{accession}/")
}

fn entry_db(accession: &str) -> Result<&'static str> {
    if is_interpro(accession) {
        Ok("interpro")
    } else if is_pfam(accession) {
        Ok("pfam")
    } else {
        bail!("{accession:?} is not an InterPro (IPRxxxxxx) or Pfam (PFxxxxx) accession")
    }
}

fn source_db(value: &str) -> Result<String> {
    let db = require_text(value, "source_db", 32)?.to_ascii_lowercase();
    if !db.chars().all(|c| c.is_ascii_alphanumeric()) {
        bail!("source_db must be an InterPro database slug such as interpro or pfam");
    }
    Ok(db)
}

fn pfam_accession(value: &str) -> Result<String> {
    let accession = require_text(value, "Pfam accession", 16)?.to_ascii_uppercase();
    if !is_pfam(&accession) {
        bail!("{accession:?} is not a Pfam accession (PFxxxxx)");
    }
    Ok(accession)
}

pub(crate) fn is_uniprot(value: &str) -> bool {
    let bytes = value.as_bytes();
    let n = bytes.len();
    if n != 6 && n != 10 {
        return false;
    }
    let first = bytes[0];
    if !first.is_ascii_alphabetic() {
        return false;
    }
    bytes.iter().all(|b| b.is_ascii_alphanumeric()) && bytes.iter().any(|b| b.is_ascii_digit())
}

fn is_interpro(value: &str) -> bool {
    let v = value.as_bytes();
    v.len() == 9 && v[..3].eq_ignore_ascii_case(b"IPR") && v[3..].iter().all(|b| b.is_ascii_digit())
}

fn is_pfam(value: &str) -> bool {
    let v = value.as_bytes();
    (v.len() == 7 || v.len() == 8)
        && v[..2].eq_ignore_ascii_case(b"PF")
        && v[2..].iter().all(|b| b.is_ascii_digit())
}

fn is_clan(value: &str) -> bool {
    let v = value.as_bytes();
    v.len() == 6 && v[..2].eq_ignore_ascii_case(b"CL") && v[2..].iter().all(|b| b.is_ascii_digit())
}

fn is_go_term(value: &str) -> bool {
    let v = value.as_bytes();
    v.len() == 10
        && v[..3].eq_ignore_ascii_case(b"GO:")
        && v[3..].iter().all(|b| b.is_ascii_digit())
}
