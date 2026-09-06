use super::{
    cap, join_url, json_plain, json_u64, ncbi_identity, pubchem_pug, pubchem_view,
    require_positive_id, require_range, require_text, send_json, NativeBio, PUBCHEM,
};
use anyhow::{bail, Context, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Map, Value};

const PROPERTY_TAGS: &str = "MolecularFormula,MolecularWeight,SMILES,ConnectivitySMILES,InChI,InChIKey,IUPACName,XLogP,ExactMass,TPSA,Charge,HBondDonorCount,HBondAcceptorCount,RotatableBondCount,HeavyAtomCount";

pub async fn search_compounds(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Search = serde_json::from_value(args.clone())
        .context("invalid pubchem_search_compounds arguments")?;
    let query = require_text(&args.query, "query", 8192)?;
    let namespace = match args.namespace.to_ascii_lowercase().as_str() {
        "name" | "smiles" | "inchikey" | "cid" => args.namespace.to_ascii_lowercase(),
        _ => bail!("namespace must be name, smiles, inchikey or cid"),
    };
    let max_cids = require_range(args.max_cids, 1, 100, "max_cids")?;
    let mut params = ncbi_identity(bio);
    params.push((namespace.clone(), query.to_string()));
    let url = join_url(
        &pubchem_pug(bio),
        &format!("compound/{namespace}/cids/JSON"),
    );
    let raw = match send_json(bio, PUBCHEM, Method::POST, &url, &params).await? {
        None => None,
        Some(value) => pubchem_payload(value)?,
    };
    let cids = match raw {
        None => Vec::new(),
        Some(value) => cids_from_identifier_list(&value)?,
    };
    let (page, truncated) = cap(&cids, max_cids);
    let properties = if args.with_properties && !page.is_empty() {
        properties_for(bio, &page).await?
    } else {
        Vec::new()
    };
    Ok(json!({
        "source": "PubChem",
        "query": query,
        "namespace": namespace,
        "n_cids_total": cids.len(),
        "truncated": truncated,
        "cids": page,
        "properties": properties
    }))
}

pub async fn get_compounds(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetCompounds =
        serde_json::from_value(args.clone()).context("invalid pubchem_get_compounds arguments")?;
    if args.cids.is_empty() || args.cids.len() > 50 {
        bail!("provide 1 to 50 PubChem CIDs");
    }
    for cid in &args.cids {
        require_positive_id(*cid, "cid")?;
    }
    let max_synonyms = require_range(args.max_synonyms, 1, 200, "max_synonyms")?;
    let unique = unique_cids(&args.cids);
    let duplicates = duplicate_cids(&args.cids);
    let records = properties_for(bio, &unique).await?;
    let mut by_cid = Map::new();
    for record in records {
        if let Some(cid) = json_u64(&record["CID"]) {
            by_cid.insert(cid.to_string(), record);
        }
    }
    if args.include_synonyms && !by_cid.is_empty() {
        let found: Vec<u64> = unique
            .iter()
            .copied()
            .filter(|cid| by_cid.contains_key(&cid.to_string()))
            .collect();
        let synonyms = synonyms_for(bio, &found).await?;
        for cid in found {
            let syns = synonyms.get(&cid).cloned().unwrap_or_default();
            let (page, truncated) = cap(&syns, max_synonyms);
            if let Some(record) = by_cid.get_mut(&cid.to_string()) {
                record["synonyms"] = json!(page);
                record["n_synonyms_total"] = json!(syns.len());
                record["synonyms_truncated"] = json!(truncated);
            }
        }
    }
    let mut ordered = Vec::new();
    let mut not_found = Vec::new();
    for cid in unique {
        match by_cid.remove(&cid.to_string()) {
            Some(record) => ordered.push(record),
            None => not_found.push(cid),
        }
    }
    Ok(json!({
        "source": "PubChem",
        "n_requested": args.cids.len(),
        "duplicates": duplicates,
        "records": ordered,
        "not_found": not_found
    }))
}

pub async fn similarity_search(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Similarity = serde_json::from_value(args.clone())
        .context("invalid pubchem_similarity_search arguments")?;
    let smiles = require_text(&args.smiles, "smiles", 8192)?;
    let threshold = require_range(args.threshold, 1, 100, "threshold")?;
    let max_records = require_range(args.max_records, 1, 200, "max_records")?;
    let mut params = ncbi_identity(bio);
    params.push(("smiles".into(), smiles.to_string()));
    let url = format!(
        "{}/compound/fastsimilarity_2d/smiles/cids/JSON?Threshold={threshold}&MaxRecords={max_records}",
        pubchem_pug(bio)
    );
    let raw = match send_json(bio, PUBCHEM, Method::POST, &url, &params).await? {
        None => None,
        Some(value) => pubchem_payload(value)?,
    };
    let cids = match raw {
        None => Vec::new(),
        Some(value) => cids_from_identifier_list(&value)?,
    };
    let prop_ids: Vec<u64> = cids.iter().copied().take(10).collect();
    let properties = if args.with_properties && !prop_ids.is_empty() {
        properties_for(bio, &prop_ids).await?
    } else {
        Vec::new()
    };
    Ok(json!({
        "source": "PubChem",
        "smiles": smiles,
        "threshold": threshold,
        "n_cids": cids.len(),
        "may_be_truncated": cids.len() >= max_records,
        "cids": cids,
        "properties": properties
    }))
}

pub async fn get_bioassay_summary(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Assay = serde_json::from_value(args.clone())
        .context("invalid pubchem_get_bioassay_summary arguments")?;
    let cid = require_positive_id(args.cid, "cid")?;
    let max_rows = require_range(args.max_rows, 1, 1000, "max_rows")?;
    let params = ncbi_identity(bio);
    let url = join_url(
        &pubchem_pug(bio),
        &format!("compound/cid/{cid}/assaysummary/JSON"),
    );
    let raw = match send_json(bio, PUBCHEM, Method::GET, &url, &params).await? {
        None => None,
        Some(value) => pubchem_payload(value)?,
    };
    let mut rows = match raw {
        None => Vec::new(),
        Some(value) => assay_rows(&value)?,
    };
    if args.active_only {
        rows.retain(|row| row.get("Activity Outcome").and_then(Value::as_str) == Some("Active"));
    }
    let total = rows.len();
    let (page, truncated) = cap(&rows, max_rows);
    Ok(json!({
        "source": "PubChem",
        "cid": cid,
        "url": compound_url(cid),
        "active_only": args.active_only,
        "n_rows_total": total,
        "truncated": truncated,
        "rows": page
    }))
}

pub async fn get_safety(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Safety =
        serde_json::from_value(args.clone()).context("invalid pubchem_get_safety arguments")?;
    let cid = require_positive_id(args.cid, "cid")?;
    let mut params = ncbi_identity(bio);
    params.push(("heading".into(), "GHS Classification".into()));
    let url = join_url(&pubchem_view(bio), &format!("data/compound/{cid}/JSON"));
    let raw = match send_json(bio, PUBCHEM, Method::GET, &url, &params).await? {
        None => None,
        Some(value) => pubchem_payload(value)?,
    };
    let ghs = raw.as_ref().and_then(ghs_from_record);
    Ok(json!({
        "source": "PubChem",
        "cid": cid,
        "url": compound_url(cid),
        "found": ghs.is_some(),
        "ghs": ghs
    }))
}

async fn properties_for(bio: &NativeBio, cids: &[u64]) -> Result<Vec<Value>> {
    if cids.is_empty() {
        return Ok(Vec::new());
    }
    let mut params = ncbi_identity(bio);
    params.push((
        "cid".into(),
        cids.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(","),
    ));
    let url = join_url(
        &pubchem_pug(bio),
        &format!("compound/cid/property/{PROPERTY_TAGS}/JSON"),
    );
    let raw = match send_json(bio, PUBCHEM, Method::POST, &url, &params).await? {
        None => return Ok(Vec::new()),
        Some(value) => match pubchem_payload(value)? {
            None => return Ok(Vec::new()),
            Some(value) => value,
        },
    };
    let table = raw
        .pointer("/PropertyTable/Properties")
        .and_then(Value::as_array)
        .context("PubChem omitted the property table")?;
    let mut by_cid = Map::new();
    for row in table {
        let Some(cid) = row.get("CID").and_then(json_u64) else {
            bail!("PubChem returned a property row without a CID");
        };
        // PUG can return a CID-only placeholder for an absent compound in a batch.
        if row.as_object().is_some_and(|object| object.len() == 1) {
            continue;
        }
        let mut record = row.clone();
        if let Some(object) = record.as_object_mut() {
            object.insert("url".into(), json!(compound_url(cid)));
        }
        by_cid.entry(cid.to_string()).or_insert(record);
    }
    Ok(cids
        .iter()
        .filter_map(|cid| by_cid.get(&cid.to_string()).cloned())
        .collect())
}

async fn synonyms_for(
    bio: &NativeBio,
    cids: &[u64],
) -> Result<std::collections::HashMap<u64, Vec<String>>> {
    let mut params = ncbi_identity(bio);
    params.push((
        "cid".into(),
        cids.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(","),
    ));
    let url = join_url(&pubchem_pug(bio), "compound/cid/synonyms/JSON");
    let raw = match send_json(bio, PUBCHEM, Method::POST, &url, &params).await? {
        None => return Ok(Default::default()),
        Some(value) => match pubchem_payload(value)? {
            None => return Ok(Default::default()),
            Some(value) => value,
        },
    };
    let infos = raw
        .pointer("/InformationList/Information")
        .and_then(Value::as_array)
        .context("PubChem omitted synonym records")?;
    let mut out = std::collections::HashMap::new();
    for info in infos {
        let Some(cid) = info.get("CID").and_then(json_u64) else {
            continue;
        };
        let syns = info
            .get("Synonym")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(json_plain).collect::<Vec<_>>())
            .unwrap_or_default();
        out.entry(cid).or_insert(syns);
    }
    Ok(out)
}

fn pubchem_payload(raw: Value) -> Result<Option<Value>> {
    if let Some(fault) = raw.get("Fault") {
        let code = fault.get("Code").and_then(Value::as_str).unwrap_or("");
        if code.contains("NotFound") {
            return Ok(None);
        }
        bail!("PubChem rejected the request");
    }
    Ok(Some(raw))
}

fn cids_from_identifier_list(raw: &Value) -> Result<Vec<u64>> {
    let Some(list) = raw.get("IdentifierList") else {
        bail!("PubChem omitted the CID list");
    };
    let cids = match list.get("CID") {
        None => return Ok(Vec::new()),
        Some(Value::Array(items)) => items,
        Some(value) => {
            let cid = json_u64(value).context("PubChem returned an invalid CID")?;
            return Ok(vec![cid]);
        }
    };
    cids.iter()
        .map(|value| json_u64(value).context("PubChem returned an invalid CID"))
        .collect()
}

fn assay_rows(raw: &Value) -> Result<Vec<Value>> {
    let Some(table) = raw.get("Table") else {
        bail!("PubChem omitted the assay summary table");
    };
    let columns: Vec<String> = table
        .pointer("/Columns/Column")
        .and_then(Value::as_array)
        .context("PubChem omitted assay summary columns")?
        .iter()
        .filter_map(json_plain)
        .collect();
    let mut rows = Vec::new();
    for row in super::as_object_array(table.get("Row")) {
        let cells = row
            .get("Cell")
            .and_then(Value::as_array)
            .context("PubChem returned an assay row without cells")?;
        let mut object = Map::new();
        for (column, cell) in columns.iter().zip(cells.iter()) {
            object.insert(column.clone(), cell.clone());
        }
        if let Some(aid) = object.get("AID").and_then(json_u64) {
            object.insert(
                "url".into(),
                json!(format!("https://pubchem.ncbi.nlm.nih.gov/bioassay/{aid}")),
            );
        }
        rows.push(Value::Object(object));
    }
    Ok(rows)
}

fn ghs_from_record(raw: &Value) -> Option<Value> {
    let record = raw.get("Record")?;
    let section = find_section(record.get("Section")?, "GHS Classification")?;
    let mut signals = Vec::new();
    let mut pictograms = Vec::new();
    let mut hazards = Vec::new();
    let mut precautionary = Vec::new();
    let mut notes = Vec::new();
    if let Some(infos) = section.get("Information").and_then(Value::as_array) {
        for info in infos {
            let name = info.get("Name").and_then(Value::as_str).unwrap_or("");
            let strings = info
                .pointer("/Value/StringWithMarkup")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            match name {
                "Signal" => push_strings(&mut signals, &strings),
                "Pictogram(s)" => {
                    for item in &strings {
                        if let Some(markup) = item.get("Markup").and_then(Value::as_array) {
                            for mark in markup {
                                if let Some(extra) = json_plain(&mark["Extra"]) {
                                    push_unique(&mut pictograms, extra);
                                }
                            }
                        }
                    }
                }
                "GHS Hazard Statements" => push_strings(&mut hazards, &strings),
                "Precautionary Statement Codes" => push_strings(&mut precautionary, &strings),
                "Note" => push_strings(&mut notes, &strings),
                _ => {}
            }
        }
    }
    let n_refs = record
        .get("Reference")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    Some(json!({
        "cid": record.get("RecordNumber").and_then(json_u64),
        "record_title": json_opt_title(record),
        "signals": signals,
        "pictograms": pictograms,
        "hazard_statements": hazards,
        "precautionary_statement_codes": precautionary,
        "notes": notes,
        "n_source_references": n_refs
    }))
}

fn json_opt_title(record: &Value) -> Value {
    record.get("RecordTitle").cloned().unwrap_or(Value::Null)
}

fn find_section<'a>(sections: &'a Value, heading: &str) -> Option<&'a Value> {
    let items = sections.as_array()?;
    for section in items {
        if section.get("TOCHeading").and_then(Value::as_str) == Some(heading) {
            return Some(section);
        }
        if let Some(found) = section
            .get("Section")
            .and_then(|inner| find_section(inner, heading))
        {
            return Some(found);
        }
    }
    None
}

fn push_strings(acc: &mut Vec<String>, strings: &[Value]) {
    for item in strings {
        if let Some(text) = json_plain(&item["String"]) {
            push_unique(acc, text);
        }
    }
}

fn push_unique(acc: &mut Vec<String>, item: String) {
    if !item.is_empty() && !acc.iter().any(|existing| existing == &item) {
        acc.push(item);
    }
}

fn compound_url(cid: u64) -> String {
    format!("https://pubchem.ncbi.nlm.nih.gov/compound/{cid}")
}

fn unique_cids(cids: &[u64]) -> Vec<u64> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for cid in cids {
        if seen.insert(*cid) {
            out.push(*cid);
        }
    }
    out
}

fn duplicate_cids(cids: &[u64]) -> Vec<u64> {
    let mut counts = std::collections::HashMap::new();
    for cid in cids {
        *counts.entry(*cid).or_insert(0usize) += 1;
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for cid in cids {
        if counts.get(cid).copied().unwrap_or(0) > 1 && seen.insert(*cid) {
            out.push(*cid);
        }
    }
    out
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: String,
    #[serde(default = "default_namespace")]
    namespace: String,
    #[serde(default = "default_max_cids")]
    max_cids: usize,
    #[serde(default = "default_true")]
    with_properties: bool,
}

fn default_namespace() -> String {
    "name".into()
}
fn default_max_cids() -> usize {
    25
}
fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetCompounds {
    cids: Vec<u64>,
    #[serde(default)]
    include_synonyms: bool,
    #[serde(default = "default_max_synonyms")]
    max_synonyms: usize,
}

fn default_max_synonyms() -> usize {
    30
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Similarity {
    smiles: String,
    #[serde(default = "default_threshold")]
    threshold: usize,
    #[serde(default = "default_max_records")]
    max_records: usize,
    #[serde(default)]
    with_properties: bool,
}

fn default_threshold() -> usize {
    90
}
fn default_max_records() -> usize {
    50
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Assay {
    cid: u64,
    #[serde(default)]
    active_only: bool,
    #[serde(default = "default_max_rows")]
    max_rows: usize,
}

fn default_max_rows() -> usize {
    100
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Safety {
    cid: u64,
}
