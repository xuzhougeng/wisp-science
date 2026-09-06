use super::{
    api_base, as_f64, as_u64, bound_int, json_string, listify, object_field, path_segment,
    require_ok, send_json, string_list, text_field, unique_ids, PDB_DATA, PDB_DATA_DEFAULT,
    PDB_SEARCH, PDB_SEARCH_DEFAULT, PDB_SITE,
};
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};

const MAX_IDS: usize = 25;
const PAGE_ROWS: usize = 100;
const EXPERIMENTAL_METHODS: &[&str] = &[
    "X-RAY DIFFRACTION",
    "ELECTRON MICROSCOPY",
    "SOLUTION NMR",
    "SOLID-STATE NMR",
    "NEUTRON DIFFRACTION",
    "ELECTRON CRYSTALLOGRAPHY",
    "FIBER DIFFRACTION",
    "POWDER DIFFRACTION",
    "SOLUTION SCATTERING",
    "EPR",
    "INFRARED SPECTROSCOPY",
    "FLUORESCENCE TRANSFER",
    "THEORETICAL MODEL",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Search {
    text: Option<String>,
    organism: Option<String>,
    taxonomy_id: Option<i64>,
    uniprot_accession: Option<String>,
    experimental_method: Option<String>,
    max_resolution_angstrom: Option<f64>,
    ligand_comp_id: Option<String>,
    #[serde(default)]
    include_computed_models: bool,
    #[serde(default = "default_search_rows")]
    max_rows: u32,
}

fn default_search_rows() -> u32 {
    100
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetStructures {
    pdb_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetEntities {
    pdb_id: String,
    entity_ids: Option<Vec<String>>,
    #[serde(default)]
    include_sequences: bool,
    #[serde(default = "default_max_bytes")]
    max_bytes: u32,
}

fn default_max_bytes() -> u32 {
    400_000
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetLigands {
    pdb_id: String,
    #[serde(default = "default_max_ligands")]
    max_ligands: u32,
}

fn default_max_ligands() -> u32 {
    25
}

pub(crate) fn fold_pdb_id(raw: &str) -> Result<Option<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() < 4 || trimmed.len() > 32 {
        bail!("PDB identifier {trimmed:?} must be 4–32 characters");
    }
    if !trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        bail!("PDB identifier {trimmed:?} contains unsupported characters");
    }
    Ok(Some(trimmed.to_ascii_uppercase()))
}

pub(crate) fn build_search_query(search: &Search) -> Result<Value> {
    let mut nodes = Vec::new();
    if let Some(text) = search
        .text
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        if text.len() > 256 {
            bail!("text must be at most 256 characters");
        }
        nodes.push(json!({
            "type": "terminal",
            "service": "full_text",
            "parameters": {"value": text}
        }));
    }
    if let Some(organism) = search
        .organism
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        nodes.push(text_node(
            "rcsb_entity_source_organism.taxonomy_lineage.name",
            "exact_match",
            json!(organism),
        ));
    }
    if let Some(taxid) = search.taxonomy_id {
        if taxid < 1 {
            bail!("taxonomy_id must be a positive NCBI taxid");
        }
        nodes.push(text_node(
            "rcsb_entity_source_organism.ncbi_taxonomy_id",
            "equals",
            json!(taxid),
        ));
    }
    if let Some(accession) = search
        .uniprot_accession
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        nodes.push(text_node(
            "rcsb_polymer_entity_container_identifiers.reference_sequence_identifiers.database_accession",
            "exact_match",
            json!(accession.to_ascii_uppercase()),
        ));
        nodes.push(text_node(
            "rcsb_polymer_entity_container_identifiers.reference_sequence_identifiers.database_name",
            "exact_match",
            json!("UniProt"),
        ));
    }
    if let Some(method) = search
        .experimental_method
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        let upper = method.to_ascii_uppercase();
        if !EXPERIMENTAL_METHODS.iter().any(|known| *known == upper) {
            bail!(
                "unknown experimental_method {method:?}; one of: {}",
                EXPERIMENTAL_METHODS.join(", ")
            );
        }
        nodes.push(text_node("exptl.method", "exact_match", json!(upper)));
    }
    if let Some(resolution) = search.max_resolution_angstrom {
        if !resolution.is_finite() || resolution <= 0.0 {
            bail!("max_resolution_angstrom must be a positive finite number");
        }
        nodes.push(text_node(
            "rcsb_entry_info.resolution_combined",
            "less_or_equal",
            json!(resolution),
        ));
    }
    if let Some(ligand) = search
        .ligand_comp_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        nodes.push(text_node(
            "rcsb_nonpolymer_entity_container_identifiers.nonpolymer_comp_id",
            "exact_match",
            json!(ligand.to_ascii_uppercase()),
        ));
    }
    if nodes.is_empty() {
        bail!(
            "at least one search criterion is required (text, organism, taxonomy_id, uniprot_accession, experimental_method, max_resolution_angstrom or ligand_comp_id)"
        );
    }
    if nodes.len() == 1 {
        Ok(nodes.pop().unwrap())
    } else {
        Ok(json!({"type": "group", "logical_operator": "and", "nodes": nodes}))
    }
}

fn text_node(attribute: &str, operator: &str, value: Value) -> Value {
    json!({
        "type": "terminal",
        "service": "text",
        "parameters": {
            "attribute": attribute,
            "operator": operator,
            "value": value
        }
    })
}

pub(crate) async fn search_structures(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Search =
        serde_json::from_value(args.clone()).context("invalid PDB search arguments")?;
    let cap = bound_int(args.max_rows, 1, 1000, "max_rows")?;
    let query = build_search_query(&args)?;
    let mut content_types = vec!["experimental"];
    if args.include_computed_models {
        content_types.push("computational");
    }
    let endpoint = api_base(bio, "PDB_SEARCH_URL", PDB_SEARCH_DEFAULT);
    let mut records = Vec::new();
    let mut total_count: Option<u64> = None;
    let mut start = 0usize;
    loop {
        let rows = PAGE_ROWS.min(cap.saturating_sub(records.len()));
        if rows == 0 {
            break;
        }
        let payload = json!({
            "query": query,
            "return_type": "entry",
            "request_options": {
                "paginate": {"start": start, "rows": rows},
                "results_content_type": content_types
            }
        });
        let (status, body) = send_json(
            bio,
            PDB_SEARCH,
            Method::GET,
            &endpoint,
            &[("json".into(), payload.to_string())],
        )
        .await?;
        if status == StatusCode::NO_CONTENT || (status.is_success() && body.is_none()) {
            if records.is_empty() {
                total_count = Some(0);
            }
            break;
        }
        require_ok(PDB_SEARCH, status)?;
        let body = body.context("RCSB PDB Search returned an empty body")?;
        let page_total = body
            .get("total_count")
            .and_then(as_u64)
            .context("RCSB PDB Search response lacked total_count")?;
        match total_count {
            None => total_count = Some(page_total),
            Some(previous) if previous != page_total => {
                bail!("RCSB PDB Search total_count changed mid-page ({previous} → {page_total})")
            }
            Some(_) => {}
        }
        let page = body
            .get("result_set")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if page.is_empty() && page_total > start as u64 {
            bail!("RCSB PDB Search returned an empty page before total_count was satisfied");
        }
        for hit in page {
            let id = text_field(&hit, &["identifier", "pdb_id"])
                .context("RCSB PDB Search hit lacked identifier")?;
            records.push(json!({
                "pdb_id": id,
                "score": hit.get("score").cloned().unwrap_or(Value::Null),
                "url": structure_url(&id)
            }));
        }
        start = records.len();
        if records.len() >= cap || records.len() as u64 >= page_total {
            break;
        }
    }
    let total = total_count.unwrap_or(records.len() as u64);
    let truncated = total > records.len() as u64;
    Ok(json!({
        "source": "RCSB PDB",
        "source_url": PDB_SITE,
        "search_api": PDB_SEARCH_DEFAULT,
        "query": {
            "text": args.text,
            "organism": args.organism,
            "taxonomy_id": args.taxonomy_id,
            "uniprot_accession": args.uniprot_accession,
            "experimental_method": args.experimental_method,
            "max_resolution_angstrom": args.max_resolution_angstrom,
            "ligand_comp_id": args.ligand_comp_id,
            "include_computed_models": args.include_computed_models
        },
        "max_rows": cap,
        "total_count": total,
        "returned": records.len(),
        "truncated": truncated,
        "records": records
    }))
}

pub(crate) async fn get_structures(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetStructures =
        serde_json::from_value(args.clone()).context("invalid PDB structure lookup arguments")?;
    let ids = unique_ids(&args.pdb_ids, MAX_IDS, "PDB identifier", fold_pdb_id)?;
    let mut records = Vec::new();
    for pdb_id in &ids.unique {
        records.push(fetch_entry(bio, pdb_id).await?);
    }
    Ok(json!({
        "source": "RCSB PDB",
        "source_url": PDB_SITE,
        "data_api": PDB_DATA_DEFAULT,
        "n_requested": ids.requested,
        "n_unique": ids.unique.len(),
        "n_blank_skipped": ids.n_blank,
        "n_duplicate_skipped": ids.n_duplicate,
        "records": records
    }))
}

pub(crate) async fn get_entities(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetEntities =
        serde_json::from_value(args.clone()).context("invalid PDB entity lookup arguments")?;
    let pdb_id = fold_pdb_id(&args.pdb_id)?.context("pdb_id is required")?;
    let max_bytes = bound_int(args.max_bytes, 1, 400_000, "max_bytes")?;
    let entry = data_object(bio, &["entry", &pdb_id])
        .await?
        .with_context(|| format!("PDB entry {pdb_id} was not found"))?;
    let ids_block = object_field(&entry, "rcsb_entry_container_identifiers")
        .map(|ids| Value::Object(ids.clone()))
        .unwrap_or(json!({}));
    let all_ids = string_list(&ids_block, &["polymer_entity_ids"]);
    let (selected, n_polymer, truncated) = match args.entity_ids.as_ref() {
        None => {
            let selected: Vec<String> = all_ids.iter().take(MAX_IDS).cloned().collect();
            (selected, Some(all_ids.len()), all_ids.len() > MAX_IDS)
        }
        Some(requested) => {
            let ids = unique_ids(requested, MAX_IDS, "polymer entity id", |raw| {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    Ok(None)
                } else if trimmed.len() > 8
                    || !trimmed
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    bail!("polymer entity id {trimmed:?} is not a PDB entity identifier");
                } else {
                    Ok(Some(trimmed.to_string()))
                }
            })?;
            (ids.unique, Some(all_ids.len()), false)
        }
    };
    let mut records = Vec::new();
    let mut not_found = Vec::new();
    for entity_id in &selected {
        match data_object(bio, &["polymer_entity", &pdb_id, entity_id]).await? {
            None => not_found.push(entity_id.clone()),
            Some(raw) => records.push(parse_polymer_entity(&raw, args.include_sequences)),
        }
    }
    let mut sequences_omitted = Value::Null;
    if args.include_sequences {
        let total: usize = records
            .iter()
            .filter_map(|record| record.get("sequence").and_then(Value::as_str))
            .map(|seq| seq.len())
            .sum();
        if total > max_bytes {
            for record in &mut records {
                if let Value::Object(map) = record {
                    map.remove("sequence");
                }
            }
            sequences_omitted = json!(format!(
                "combined sequences are {total} bytes > max_bytes={max_bytes}; request fewer entity_ids or a larger max_bytes"
            ));
        }
    }
    Ok(json!({
        "source": "RCSB PDB",
        "source_url": PDB_SITE,
        "pdb_id": pdb_id,
        "url": structure_url(&pdb_id),
        "n_polymer_entities": n_polymer,
        "polymer_entity_ids": selected,
        "truncated": truncated,
        "not_found": not_found,
        "sequences_omitted": sequences_omitted,
        "records": records
    }))
}

pub(crate) async fn get_ligands(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetLigands =
        serde_json::from_value(args.clone()).context("invalid PDB ligand lookup arguments")?;
    let pdb_id = fold_pdb_id(&args.pdb_id)?.context("pdb_id is required")?;
    let cap = bound_int(args.max_ligands, 1, 25, "max_ligands")?;
    let entry = data_object(bio, &["entry", &pdb_id])
        .await?
        .with_context(|| format!("PDB entry {pdb_id} was not found"))?;
    let np_ids = object_field(&entry, "rcsb_entry_container_identifiers")
        .map(|ids| string_list(&Value::Object(ids.clone()), &["non_polymer_entity_ids"]))
        .unwrap_or_default();
    let use_ids: Vec<String> = np_ids.iter().take(cap).cloned().collect();
    let mut entities = Vec::new();
    let mut comp_ids = Vec::new();
    for entity_id in &use_ids {
        match data_object(bio, &["nonpolymer_entity", &pdb_id, entity_id]).await? {
            None => entities.push(
                json!({"entity_id": entity_id, "comp_id": Value::Null, "error": "not_found"}),
            ),
            Some(raw) => {
                let ids = object_field(&raw, "rcsb_nonpolymer_entity_container_identifiers")
                    .map(|map| Value::Object(map.clone()))
                    .unwrap_or(json!({}));
                let ent = object_field(&raw, "rcsb_nonpolymer_entity")
                    .map(|map| Value::Object(map.clone()))
                    .unwrap_or(json!({}));
                let comp_id = text_field(&ids, &["nonpolymer_comp_id"]);
                if let Some(comp_id) = &comp_id {
                    if !comp_ids.iter().any(|id| id == comp_id) {
                        comp_ids.push(comp_id.clone());
                    }
                }
                entities.push(json!({
                    "entity_id": text_field(&ids, &["entity_id"]).unwrap_or_else(|| entity_id.clone()),
                    "comp_id": json!(comp_id),
                    "description": json_string(ent.get("pdbx_description")),
                    "n_copies_deposited": ent.get("pdbx_number_of_molecules").cloned().unwrap_or(Value::Null),
                    "auth_asym_ids": string_list(&ids, &["auth_asym_ids"])
                }));
            }
        }
    }
    let mut comps = serde_json::Map::new();
    for comp_id in &comp_ids {
        let parsed = match data_object(bio, &["chemcomp", comp_id]).await? {
            None => json!({"comp_id": comp_id, "error": "not_found"}),
            Some(raw) => parse_chem_comp(&raw),
        };
        comps.insert(comp_id.clone(), parsed);
    }
    let ligands: Vec<Value> = entities
        .into_iter()
        .map(|mut entity| {
            if let Some(comp_id) = entity.get("comp_id").and_then(Value::as_str) {
                entity["chem_comp"] = comps.get(comp_id).cloned().unwrap_or(Value::Null);
            } else {
                entity["chem_comp"] = Value::Null;
            }
            entity
        })
        .collect();
    Ok(json!({
        "source": "RCSB PDB",
        "source_url": PDB_SITE,
        "pdb_id": pdb_id,
        "url": structure_url(&pdb_id),
        "n_nonpolymer_entities": np_ids.len(),
        "n_returned": ligands.len(),
        "truncated": np_ids.len() > ligands.len(),
        "ligands": ligands
    }))
}

async fn fetch_entry(bio: &NativeBio, pdb_id: &str) -> Result<Value> {
    match data_object(bio, &["entry", pdb_id]).await? {
        None => Ok(json!({
            "pdb_id": pdb_id,
            "error": "not_found",
            "url": structure_url(pdb_id)
        })),
        Some(raw) => Ok(parse_entry(&raw)),
    }
}

async fn data_object(bio: &NativeBio, segments: &[&str]) -> Result<Option<Value>> {
    let base = api_base(bio, "PDB_DATA_URL", PDB_DATA_DEFAULT);
    let path: Vec<String> = segments.iter().map(|s| path_segment(s)).collect();
    let url = format!("{base}/{}", path.join("/"));
    let (status, body) = send_json(bio, PDB_DATA, Method::GET, &url, &[]).await?;
    if status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    require_ok(PDB_DATA, status)?;
    Ok(Some(body.context("RCSB PDB Data returned an empty body")?))
}

fn parse_entry(raw: &Value) -> Value {
    let info = object_field(raw, "rcsb_entry_info")
        .map(|map| Value::Object(map.clone()))
        .unwrap_or(json!({}));
    let acc = object_field(raw, "rcsb_accession_info")
        .map(|map| Value::Object(map.clone()))
        .unwrap_or(json!({}));
    let ids = object_field(raw, "rcsb_entry_container_identifiers")
        .map(|map| Value::Object(map.clone()))
        .unwrap_or(json!({}));
    let methods: Vec<Value> = listify(raw.get("exptl"))
        .into_iter()
        .filter_map(|exptl| text_field(exptl, &["method"]).map(Value::from))
        .collect();
    let resolutions: Vec<Value> = listify(info.get("resolution_combined"))
        .into_iter()
        .filter_map(as_f64)
        .map(|n| json!(n))
        .collect();
    let resolution = resolutions.iter().filter_map(as_f64).fold(None, |best, n| {
        Some(best.map(|b: f64| b.min(n)).unwrap_or(n))
    });
    let pdb_id = text_field(raw, &["rcsb_id"]).unwrap_or_default();
    json!({
        "pdb_id": pdb_id,
        "url": structure_url(&pdb_id),
        "title": json_string(object_field(raw, "struct").and_then(|s| s.get("title"))),
        "experimental_methods": methods,
        "resolution_angstrom": json!(resolution),
        "resolutions_combined": resolutions,
        "structure_determination_methodology": json_string(info.get("structure_determination_methodology")),
        "deposit_date": json_string(acc.get("deposit_date")),
        "initial_release_date": json_string(acc.get("initial_release_date")),
        "revision_date": json_string(acc.get("revision_date")),
        "status_code": json_string(acc.get("status_code")),
        "molecular_weight_kda": info.get("molecular_weight").cloned().unwrap_or(Value::Null),
        "assembly_count": info.get("assembly_count").cloned().unwrap_or(Value::Null),
        "polymer_entity_count": info.get("polymer_entity_count").cloned().unwrap_or(Value::Null),
        "polymer_entity_count_protein": info.get("polymer_entity_count_protein").cloned().unwrap_or(Value::Null),
        "polymer_entity_count_dna": info.get("polymer_entity_count_DNA").cloned().unwrap_or(Value::Null),
        "polymer_entity_count_rna": info.get("polymer_entity_count_RNA").cloned().unwrap_or(Value::Null),
        "nonpolymer_entity_count": info.get("nonpolymer_entity_count").cloned().unwrap_or(Value::Null),
        "polymer_composition": json_string(info.get("polymer_composition")),
        "ligand_comp_ids": string_list(&info, &["nonpolymer_bound_components"]),
        "polymer_entity_ids": string_list(&ids, &["polymer_entity_ids"]),
        "nonpolymer_entity_ids": string_list(&ids, &["non_polymer_entity_ids"]),
        "citation": parse_citation(raw.get("rcsb_primary_citation"))
    })
}

fn parse_citation(node: Option<&Value>) -> Value {
    let Some(Value::Object(cit)) = node else {
        return Value::Null;
    };
    if cit.is_empty() {
        return Value::Null;
    }
    json!({
        "title": json_string(cit.get("title")),
        "journal": json_string(cit.get("rcsb_journal_abbrev").or(cit.get("journal_abbrev"))),
        "year": cit.get("year").cloned().unwrap_or(Value::Null),
        "authors": string_list(&Value::Object(cit.clone()), &["rcsb_authors"]),
        "pubmed_id": cit.get("pdbx_database_id_PubMed").cloned().unwrap_or(Value::Null),
        "doi": json_string(cit.get("pdbx_database_id_DOI"))
    })
}

fn parse_polymer_entity(raw: &Value, include_sequence: bool) -> Value {
    let ent = object_field(raw, "rcsb_polymer_entity")
        .map(|map| Value::Object(map.clone()))
        .unwrap_or(json!({}));
    let ids = object_field(raw, "rcsb_polymer_entity_container_identifiers")
        .map(|map| Value::Object(map.clone()))
        .unwrap_or(json!({}));
    let poly = object_field(raw, "entity_poly")
        .map(|map| Value::Object(map.clone()))
        .unwrap_or(json!({}));
    let organisms: Vec<Value> = listify(raw.get("rcsb_entity_source_organism"))
        .into_iter()
        .map(|org| {
            json!({
                "scientific_name": json_string(org.get("scientific_name")),
                "ncbi_taxonomy_id": org.get("ncbi_taxonomy_id").cloned().unwrap_or(Value::Null)
            })
        })
        .collect();
    let refs: Vec<Value> = listify(ids.get("reference_sequence_identifiers"))
        .into_iter()
        .map(|item| {
            json!({
                "database_name": json_string(item.get("database_name")),
                "database_accession": json_string(item.get("database_accession")),
                "entity_sequence_coverage": item.get("entity_sequence_coverage").cloned().unwrap_or(Value::Null),
                "reference_sequence_coverage": item.get("reference_sequence_coverage").cloned().unwrap_or(Value::Null)
            })
        })
        .collect();
    let aligned: Vec<Value> = listify(raw.get("rcsb_polymer_entity_align"))
        .into_iter()
        .filter(|item| {
            text_field(item, &["reference_database_name"])
                .is_some_and(|name| name.eq_ignore_ascii_case("UniProt"))
        })
        .map(|item| {
            json!({
                "accession": json_string(item.get("reference_database_accession")),
                "regions": item.get("aligned_regions").cloned().unwrap_or_else(|| json!([]))
            })
        })
        .collect();
    let mut record = json!({
        "rcsb_id": json_string(raw.get("rcsb_id")),
        "entry_id": json_string(ids.get("entry_id")),
        "entity_id": json_string(ids.get("entity_id")),
        "description": json_string(ent.get("pdbx_description")),
        "polymer_type": json_string(poly.get("rcsb_entity_polymer_type")),
        "polymer_type_detail": json_string(poly.get("type")),
        "sequence_length": poly.get("rcsb_sample_sequence_length").cloned().unwrap_or(Value::Null),
        "mutation_count": poly.get("rcsb_mutation_count").cloned().unwrap_or(Value::Null),
        "n_copies_deposited": ent.get("pdbx_number_of_molecules").cloned().unwrap_or(Value::Null),
        "molecular_weight_kda": ent.get("formula_weight").cloned().unwrap_or(Value::Null),
        "asym_ids": string_list(&ids, &["asym_ids"]),
        "auth_asym_ids": string_list(&ids, &["auth_asym_ids"]),
        "source_organisms": organisms,
        "uniprot_ids": string_list(&ids, &["uniprot_ids"]),
        "reference_sequence_identifiers": refs,
        "uniprot_aligned_regions": aligned
    });
    if include_sequence {
        record["sequence"] = json_string(poly.get("pdbx_seq_one_letter_code_can"));
    }
    record
}

fn parse_chem_comp(raw: &Value) -> Value {
    let comp = object_field(raw, "chem_comp")
        .map(|map| Value::Object(map.clone()))
        .unwrap_or(json!({}));
    let desc = object_field(raw, "rcsb_chem_comp_descriptor")
        .map(|map| Value::Object(map.clone()))
        .unwrap_or(json!({}));
    json!({
        "comp_id": json_string(comp.get("id")),
        "name": json_string(comp.get("name")),
        "formula": json_string(comp.get("formula")),
        "formula_weight": comp.get("formula_weight").cloned().unwrap_or(Value::Null),
        "formal_charge": comp.get("pdbx_formal_charge").cloned().unwrap_or(Value::Null),
        "type": json_string(comp.get("type")),
        "inchikey": json_string(desc.get("InChIKey")),
        "smiles": json_string(desc.get("SMILES_stereo").or(desc.get("SMILES")))
    })
}

fn structure_url(pdb_id: &str) -> String {
    format!("{PDB_SITE}/structure/{}", path_segment(pdb_id))
}
