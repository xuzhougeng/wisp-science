//! Native ChEMBL domain, implemented from:
//! - ChEMBL Data Web Services (https://www.ebi.ac.uk/chembl/api/data/docs)
//! - ChEMBL interface documentation
//!   (https://chembl.gitbook.io/chembl-interface-documentation/web-services/chembl-data-web-services)
//!
//! References reviewed 2026-09-06. JSON is requested with the `.json` suffix.
//! Tests use invented records.

use crate::http::Source;
use crate::NativeBio;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::time::Duration;
use wisp_llm::ToolSchema;

const DATA: &str = "https://www.ebi.ac.uk/chembl/api/data";
const WEB: &str = "https://www.ebi.ac.uk/chembl";
const CHEMBL: Source = Source("ChEMBL", Duration::from_millis(200));
const MAX_LIMIT: u32 = 100;
const MAX_SYNONYMS: usize = 25;
const MAX_COMPONENTS: usize = 20;
const ACTIVITY_TYPES: &[&str] = &[
    "IC50", "EC50", "XC50", "Ki", "Kd", "AC50", "GI50", "ED50", "Potency",
];
const UNITS: &[&str] = &["nM", "uM", "mM", "pM", "M"];
const TARGET_TYPES: &[&str] = &[
    "SINGLE PROTEIN",
    "PROTEIN COMPLEX",
    "PROTEIN FAMILY",
    "PROTEIN-PROTEIN INTERACTION",
    "CHIMERIC PROTEIN",
    "SELECTIVITY GROUP",
    "ORGANISM",
    "TISSUE",
    "CELL-LINE",
    "NUCLEIC-ACID",
    "SUBCELLULAR",
    "UNKNOWN",
];

fn default_limit() -> u32 {
    20
}

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        (
            "chembl",
            ToolSchema::new(
                "compound_search",
                "Search ChEMBL molecules by name or synonym, molecule identifier, or SMILES. With smiles and similarity_threshold the similarity resource is used (Tanimoto percent, 40-100); smiles alone uses substructure. Returns a bounded page of molecules with compound report-card URLs. Calculated properties are not experimental ADMET measurements.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "name": {"type": "string", "minLength": 1, "maxLength": 256, "description": "Preferred name or synonym substring (case-insensitive)."},
                        "chembl_id": {"type": "string", "description": "Molecule ChEMBL identifier, for example CHEMBL25."},
                        "smiles": {"type": "string", "minLength": 1, "maxLength": 2048, "description": "Query SMILES for similarity or substructure search."},
                        "similarity_threshold": {"type": "integer", "minimum": 40, "maximum": 100, "description": "Tanimoto percent cutoff. Requires smiles."},
                        "max_phase": {"type": "integer", "minimum": 0, "maximum": 4, "description": "Clinical max_phase filter. 4 is approved."},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20}
                    }
                }),
            ),
        ),
        (
            "chembl",
            ToolSchema::new(
                "drug_search",
                "Find ChEMBL parent drugs linked to a disease indication through EFO term or MeSH heading. Optional filters restrict to an approved indication phase, a parent molecule identifier, or a preferred-name substring. Returns a bounded page of indication-linked drugs with report-card URLs, not the complete indication table.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["indication"],
                    "properties": {
                        "indication": {"type": "string", "minLength": 1, "maxLength": 256, "description": "Disease term matched against EFO, then MeSH heading."},
                        "only_approved": {"type": "boolean", "default": false, "description": "Restrict to max_phase_for_ind=4."},
                        "max_phase": {"type": "integer", "minimum": 0, "maximum": 4, "description": "Minimum max_phase_for_ind when only_approved is false."},
                        "molecule_chembl_id": {"type": "string", "description": "Parent molecule ChEMBL identifier."},
                        "drug_name": {"type": "string", "minLength": 1, "maxLength": 256, "description": "Case-insensitive preferred-name substring applied to the returned page."},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20}
                    }
                }),
            ),
        ),
        (
            "chembl",
            ToolSchema::new(
                "get_admet",
                "Retrieve ChEMBL calculated molecule properties for one molecule identifier: molecular weight, ALogP, polar surface area, hydrogen-bond counts, rotatable bonds, QED and Rule-of-Five violations. These are structure-derived descriptors from the molecule resource, not experimental ADMET assays.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["molecule_chembl_id"],
                    "properties": {
                        "molecule_chembl_id": {"type": "string", "description": "Molecule ChEMBL identifier, for example CHEMBL25."}
                    }
                }),
            ),
        ),
        (
            "chembl",
            ToolSchema::new(
                "get_bioactivity",
                "Retrieve a bounded page of ChEMBL activity records for a molecule and/or target identifier. Optional filters cover standard type, pChEMBL, value range and units. pChEMBL is ChEMBL's -log10 nanomolar potency for qualifying measurements. Unfiltered download of the activity table is rejected.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "molecule_chembl_id": {"type": "string", "description": "Molecule ChEMBL identifier."},
                        "target_chembl_id": {"type": "string", "description": "Target ChEMBL identifier."},
                        "activity_type": {"type": "string", "enum": ["IC50", "EC50", "XC50", "Ki", "Kd", "AC50", "GI50", "ED50", "Potency"]},
                        "min_pchembl": {"type": "number", "minimum": 0, "maximum": 14, "description": "Minimum pchembl_value."},
                        "min_value": {"type": "number", "description": "Minimum standard_value in standard units."},
                        "max_value": {"type": "number", "description": "Maximum standard_value in standard units."},
                        "unit": {"type": "string", "enum": ["nM", "uM", "mM", "pM", "M"]},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20}
                    }
                }),
            ),
        ),
        (
            "chembl",
            ToolSchema::new(
                "get_mechanism",
                "Retrieve curated ChEMBL mechanism-of-action records for a molecule and/or target identifier. Mechanism rows are aggregated on parent molecules; a molecule identifier is retried as parent_molecule_chembl_id when the direct lookup is empty. Unfiltered download of the mechanism table is rejected.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "molecule_chembl_id": {"type": "string", "description": "Molecule or parent molecule ChEMBL identifier."},
                        "target_chembl_id": {"type": "string", "description": "Target ChEMBL identifier."},
                        "action_type": {"type": "string", "minLength": 1, "maxLength": 80, "description": "Mechanism action_type, for example INHIBITOR or AGONIST."},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20}
                    }
                }),
            ),
        ),
        (
            "chembl",
            ToolSchema::new(
                "target_search",
                "Search ChEMBL targets by identifier, preferred name, gene symbol, organism or target type. Gene symbols match target-component synonyms (iexact). Returns a bounded page with UniProt accessions, gene symbols and target report-card URLs. Unfiltered download of the target table is rejected.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "target_chembl_id": {"type": "string", "description": "Target ChEMBL identifier, for example CHEMBL203."},
                        "target_name": {"type": "string", "minLength": 1, "maxLength": 256, "description": "Preferred-name substring (case-insensitive)."},
                        "gene_symbol": {"type": "string", "minLength": 1, "maxLength": 64, "description": "Exact gene symbol against component synonyms."},
                        "organism": {"type": "string", "minLength": 1, "maxLength": 128, "description": "Organism name substring, for example Homo sapiens."},
                        "target_type": {"type": "string", "enum": [
                            "SINGLE PROTEIN", "PROTEIN COMPLEX", "PROTEIN FAMILY",
                            "PROTEIN-PROTEIN INTERACTION", "CHIMERIC PROTEIN", "SELECTIVITY GROUP",
                            "ORGANISM", "TISSUE", "CELL-LINE", "NUCLEIC-ACID", "SUBCELLULAR", "UNKNOWN"
                        ]},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20}
                    }
                }),
            ),
        ),
    ]
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(45), dispatch(bio, name, args))
        .await
        .map_err(|_| anyhow!("ChEMBL request exceeded 45 seconds"))?
}

async fn dispatch(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "compound_search" => {
            let args: CompoundSearch = serde_json::from_value(args.clone())
                .context("invalid compound_search arguments")?;
            compound_search(bio, args).await
        }
        "drug_search" => {
            let args: DrugSearch =
                serde_json::from_value(args.clone()).context("invalid drug_search arguments")?;
            drug_search(bio, args).await
        }
        "get_admet" => {
            let args: GetAdmet =
                serde_json::from_value(args.clone()).context("invalid get_admet arguments")?;
            get_admet(bio, args).await
        }
        "get_bioactivity" => {
            let args: GetBioactivity = serde_json::from_value(args.clone())
                .context("invalid get_bioactivity arguments")?;
            get_bioactivity(bio, args).await
        }
        "get_mechanism" => {
            let args: GetMechanism =
                serde_json::from_value(args.clone()).context("invalid get_mechanism arguments")?;
            get_mechanism(bio, args).await
        }
        "target_search" => {
            let args: TargetSearch =
                serde_json::from_value(args.clone()).context("invalid target_search arguments")?;
            target_search(bio, args).await
        }
        _ => bail!("unknown native biological tool: {name}"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompoundSearch {
    name: Option<String>,
    chembl_id: Option<String>,
    smiles: Option<String>,
    similarity_threshold: Option<u32>,
    max_phase: Option<u8>,
    #[serde(default = "default_limit")]
    limit: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DrugSearch {
    indication: String,
    #[serde(default)]
    only_approved: bool,
    max_phase: Option<u8>,
    molecule_chembl_id: Option<String>,
    drug_name: Option<String>,
    #[serde(default = "default_limit")]
    limit: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetAdmet {
    molecule_chembl_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetBioactivity {
    molecule_chembl_id: Option<String>,
    target_chembl_id: Option<String>,
    activity_type: Option<String>,
    min_pchembl: Option<f64>,
    min_value: Option<f64>,
    max_value: Option<f64>,
    unit: Option<String>,
    #[serde(default = "default_limit")]
    limit: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetMechanism {
    molecule_chembl_id: Option<String>,
    target_chembl_id: Option<String>,
    action_type: Option<String>,
    #[serde(default = "default_limit")]
    limit: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetSearch {
    target_chembl_id: Option<String>,
    target_name: Option<String>,
    gene_symbol: Option<String>,
    organism: Option<String>,
    target_type: Option<String>,
    #[serde(default = "default_limit")]
    limit: u32,
}

async fn compound_search(bio: &NativeBio, args: CompoundSearch) -> Result<Value> {
    let limit = bound_limit(args.limit)?;
    let max_phase = bound_phase(args.max_phase)?;
    let chembl_id = optional_chembl_id(args.chembl_id.as_deref())?;
    let name = optional_query("name", args.name.as_deref(), 256)?;
    let smiles = optional_smiles(args.smiles.as_deref())?;
    if args.similarity_threshold.is_some() && smiles.is_none() {
        bail!("similarity_threshold requires smiles");
    }
    if chembl_id.is_none() && name.is_none() && smiles.is_none() {
        bail!("compound_search requires name, chembl_id, or smiles");
    }
    let (mut page, kind) = if let Some(id) = chembl_id {
        let mut params = list_params(limit, Some("molecule_chembl_id"));
        params.push(("molecule_chembl_id".into(), id));
        (
            list_page(bio, "molecule.json", "molecules", params).await?,
            "identifier",
        )
    } else if let Some(smiles) = smiles {
        let encoded = path_segment(smiles);
        if let Some(threshold) = args.similarity_threshold {
            if !(40..=100).contains(&threshold) {
                bail!("similarity_threshold must be between 40 and 100");
            }
            let path = format!("similarity/{encoded}/{threshold}.json");
            (
                list_page(bio, &path, "molecules", list_params(limit, None)).await?,
                "similarity",
            )
        } else {
            let path = format!("substructure/{encoded}.json");
            (
                list_page(bio, &path, "molecules", list_params(limit, None)).await?,
                "substructure",
            )
        }
    } else if let Some(name) = name {
        let mut params = list_params(limit, Some("molecule_chembl_id"));
        params.push((
            "molecule_synonyms__molecule_synonym__icontains".into(),
            name.to_string(),
        ));
        if let Some(phase) = max_phase {
            params.push(("max_phase".into(), phase.to_string()));
        }
        let page = list_page(bio, "molecule.json", "molecules", params).await?;
        if page.records.is_empty() && page.total == Some(0) {
            let mut params = list_params(limit, Some("molecule_chembl_id"));
            params.push(("pref_name__icontains".into(), name.to_string()));
            if let Some(phase) = max_phase {
                params.push(("max_phase".into(), phase.to_string()));
            }
            (
                list_page(bio, "molecule.json", "molecules", params).await?,
                "name",
            )
        } else {
            (page, "name")
        }
    } else {
        unreachable!("compound_search requires a selector");
    };

    if kind == "similarity" {
        page.records.sort_by(|left, right| {
            cmp_f64_desc(number(&left["similarity"]), number(&right["similarity"])).then_with(
                || text(&left["molecule_chembl_id"]).cmp(&text(&right["molecule_chembl_id"])),
            )
        });
    } else if kind == "substructure" {
        page.records.sort_by(|left, right| {
            text(&left["molecule_chembl_id"]).cmp(&text(&right["molecule_chembl_id"]))
        });
    }

    if let Some(phase) = max_phase {
        if kind != "name" {
            page.records.retain(|molecule| {
                number(&molecule["max_phase"]).is_some_and(|value| value == f64::from(phase))
            });
        }
    }

    let compounds: Vec<Value> = page.records.iter().map(molecule_record).collect();
    Ok(json!({
        "source": "ChEMBL",
        "query_kind": kind,
        "returned": compounds.len(),
        "total": page.total_json(),
        "has_more": page.has_more,
        "compounds": compounds
    }))
}

async fn drug_search(bio: &NativeBio, args: DrugSearch) -> Result<Value> {
    let limit = bound_limit(args.limit)?;
    let indication = require_query("indication", &args.indication, 256)?;
    let max_phase = bound_phase(args.max_phase)?;
    let parent_id = optional_chembl_id(args.molecule_chembl_id.as_deref())?;
    let drug_name = optional_query("drug_name", args.drug_name.as_deref(), 256)?;

    let mut params = list_params(limit, Some("drugind_id"));
    params.push(("efo_term__icontains".into(), indication.to_string()));
    apply_indication_phase(&mut params, args.only_approved, max_phase);
    if let Some(id) = &parent_id {
        params.push(("parent_molecule_chembl_id".into(), id.clone()));
    }
    let mut page = list_page(bio, "drug_indication.json", "drug_indications", params).await?;
    let mut match_field = "efo_term";
    if page.records.is_empty() && page.total == Some(0) {
        let mut params = list_params(limit, Some("drugind_id"));
        params.push(("mesh_heading__icontains".into(), indication.to_string()));
        apply_indication_phase(&mut params, args.only_approved, max_phase);
        if let Some(id) = &parent_id {
            params.push(("parent_molecule_chembl_id".into(), id.clone()));
        }
        page = list_page(bio, "drug_indication.json", "drug_indications", params).await?;
        match_field = "mesh_heading";
    }

    let parents = unique_ids(page.records.iter().filter_map(|row| {
        text(&row["parent_molecule_chembl_id"]).or_else(|| text(&row["molecule_chembl_id"]))
    }));
    let molecules = if parents.is_empty() {
        Vec::new()
    } else {
        let mut params = list_params(parents.len() as u32, Some("molecule_chembl_id"));
        params.push(("molecule_chembl_id__in".into(), parents.join(",")));
        list_page(bio, "molecule.json", "molecules", params)
            .await?
            .records
    };
    let warnings = if parents.is_empty() {
        Vec::new()
    } else {
        let mut params = list_params(MAX_LIMIT, Some("warning_id"));
        params.push(("parent_molecule_chembl_id__in".into(), parents.join(",")));
        list_page(bio, "drug_warning.json", "drug_warnings", params)
            .await?
            .records
    };

    let mut drugs = Vec::new();
    for parent in &parents {
        let rows: Vec<&Value> = page
            .records
            .iter()
            .filter(|row| {
                text(&row["parent_molecule_chembl_id"]).as_deref() == Some(parent.as_str())
                    || (text(&row["parent_molecule_chembl_id"]).is_none()
                        && text(&row["molecule_chembl_id"]).as_deref() == Some(parent.as_str()))
            })
            .collect();
        let molecule = molecules
            .iter()
            .find(|record| text(&record["molecule_chembl_id"]).as_deref() == Some(parent.as_str()));
        let pref_name = molecule.and_then(|record| text(&record["pref_name"]));
        if let Some(needle) = drug_name {
            let haystack = pref_name.as_deref().unwrap_or("").to_ascii_lowercase();
            if !haystack.contains(&needle.to_ascii_lowercase()) {
                continue;
            }
        }
        let best_phase = rows
            .iter()
            .filter_map(|row| number(&row["max_phase_for_ind"]))
            .fold(None, |best: Option<f64>, value| {
                Some(best.map_or(value, |current| current.max(value)))
            });
        let parent_warnings: Vec<Value> = warnings
            .iter()
            .filter(|warning| {
                text(&warning["parent_molecule_chembl_id"]).as_deref() == Some(parent.as_str())
            })
            .map(warning_record)
            .collect();
        drugs.push(json!({
            "molecule_chembl_id": parent,
            "pref_name": pref_name,
            "molecule_type": molecule.map(|record| record.get("molecule_type").cloned().unwrap_or(Value::Null)).unwrap_or(Value::Null),
            "max_phase": molecule.map(|record| json_num(&record["max_phase"])).unwrap_or(Value::Null),
            "first_approval": molecule.map(|record| json_num(&record["first_approval"])).unwrap_or(Value::Null),
            "withdrawn": molecule.map(|record| json_flag(&record["withdrawn_flag"])).unwrap_or(Value::Null),
            "black_box_warning": molecule.map(|record| json_flag(&record["black_box_warning"])).unwrap_or(Value::Null),
            "best_phase_for_indication": best_phase.map(|value| json_num(&json!(value))).unwrap_or(Value::Null),
            "mesh_headings": unique_strings(rows.iter().copied(), "mesh_heading"),
            "efo_terms": unique_strings(rows.iter().copied(), "efo_term"),
            "warnings": unique_warnings(parent_warnings),
            "url": compound_url(parent)
        }));
    }

    Ok(json!({
        "source": "ChEMBL",
        "indication": indication,
        "match_field": match_field,
        "returned": drugs.len(),
        "total": page.total_json(),
        "has_more": page.has_more,
        "drugs": drugs
    }))
}

async fn get_admet(bio: &NativeBio, args: GetAdmet) -> Result<Value> {
    let id = parse_chembl_id(&args.molecule_chembl_id)?;
    let mut params = list_params(1, Some("molecule_chembl_id"));
    params.push(("molecule_chembl_id".into(), id.clone()));
    let page = list_page(bio, "molecule.json", "molecules", params).await?;
    let molecule = page.records.first();
    let properties = molecule.map(admet_properties);
    Ok(json!({
        "source": "ChEMBL",
        "found": properties.is_some(),
        "molecule_chembl_id": id,
        "url": compound_url(&id),
        "properties": properties
    }))
}

async fn get_bioactivity(bio: &NativeBio, args: GetBioactivity) -> Result<Value> {
    let limit = bound_limit(args.limit)?;
    let molecule = optional_chembl_id(args.molecule_chembl_id.as_deref())?;
    let target = optional_chembl_id(args.target_chembl_id.as_deref())?;
    if molecule.is_none() && target.is_none() {
        bail!("get_bioactivity requires molecule_chembl_id or target_chembl_id");
    }
    if let Some(min) = args.min_pchembl {
        if !(0.0..=14.0).contains(&min) || !min.is_finite() {
            bail!("min_pchembl must be between 0 and 14");
        }
    }
    if let (Some(min), Some(max)) = (args.min_value, args.max_value) {
        if min > max {
            bail!("min_value must be less than or equal to max_value");
        }
    }
    let mut params = list_params(limit, Some("activity_id"));
    if let Some(id) = molecule {
        params.push(("molecule_chembl_id".into(), id));
    }
    if let Some(id) = target {
        params.push(("target_chembl_id".into(), id));
    }
    if let Some(kind) = args.activity_type.as_deref() {
        if !ACTIVITY_TYPES.contains(&kind) {
            bail!("activity_type is not a supported ChEMBL standard_type");
        }
        params.push(("standard_type".into(), kind.to_string()));
    }
    if let Some(min) = args.min_pchembl {
        params.push(("pchembl_value__gte".into(), number_param(min)));
    }
    if let Some(min) = args.min_value {
        params.push(("standard_value__gte".into(), number_param(min)));
    }
    if let Some(max) = args.max_value {
        params.push(("standard_value__lte".into(), number_param(max)));
    }
    if let Some(unit) = args.unit.as_deref() {
        if !UNITS.contains(&unit) {
            bail!("unit must be one of nM, uM, mM, pM or M");
        }
        params.push(("standard_units".into(), unit.to_string()));
    }
    let page = list_page(bio, "activity.json", "activities", params).await?;
    let activities: Vec<Value> = page.records.iter().map(activity_record).collect();
    Ok(json!({
        "source": "ChEMBL",
        "returned": activities.len(),
        "total": page.total_json(),
        "has_more": page.has_more,
        "activities": activities
    }))
}

async fn get_mechanism(bio: &NativeBio, args: GetMechanism) -> Result<Value> {
    let limit = bound_limit(args.limit)?;
    let molecule = optional_chembl_id(args.molecule_chembl_id.as_deref())?;
    let target = optional_chembl_id(args.target_chembl_id.as_deref())?;
    let action = optional_query("action_type", args.action_type.as_deref(), 80)?;
    if molecule.is_none() && target.is_none() {
        bail!("get_mechanism requires molecule_chembl_id or target_chembl_id");
    }
    let mut params = mechanism_params(limit, molecule.as_deref(), target.as_deref(), action, false);
    let mut page = list_page(bio, "mechanism.json", "mechanisms", params).await?;
    if page.records.is_empty() && page.total == Some(0) {
        if let Some(id) = molecule.as_deref() {
            params = mechanism_params(limit, Some(id), target.as_deref(), action, true);
            page = list_page(bio, "mechanism.json", "mechanisms", params).await?;
        }
    }
    let mechanisms: Vec<Value> = page.records.iter().map(mechanism_record).collect();
    Ok(json!({
        "source": "ChEMBL",
        "returned": mechanisms.len(),
        "total": page.total_json(),
        "has_more": page.has_more,
        "mechanisms": mechanisms
    }))
}

async fn target_search(bio: &NativeBio, args: TargetSearch) -> Result<Value> {
    let limit = bound_limit(args.limit)?;
    let target_id = optional_chembl_id(args.target_chembl_id.as_deref())?;
    let target_name = optional_query("target_name", args.target_name.as_deref(), 256)?;
    let gene = optional_query("gene_symbol", args.gene_symbol.as_deref(), 64)?;
    let organism = optional_query("organism", args.organism.as_deref(), 128)?;
    if target_id.is_none()
        && target_name.is_none()
        && gene.is_none()
        && organism.is_none()
        && args.target_type.is_none()
    {
        bail!("target_search requires target_chembl_id, target_name, gene_symbol, organism, or target_type");
    }
    let mut params = list_params(limit, Some("target_chembl_id"));
    if let Some(id) = target_id {
        params.push(("target_chembl_id".into(), id));
    }
    if let Some(name) = target_name {
        params.push(("pref_name__icontains".into(), name.to_string()));
    }
    if let Some(symbol) = gene {
        params.push((
            "target_components__target_component_synonyms__component_synonym__iexact".into(),
            symbol.to_string(),
        ));
    }
    if let Some(organism) = organism {
        params.push(("organism__icontains".into(), organism.to_string()));
    }
    if let Some(kind) = args.target_type.as_deref() {
        if !TARGET_TYPES.contains(&kind) {
            bail!("target_type is not a supported ChEMBL target type");
        }
        params.push(("target_type".into(), kind.to_string()));
    }
    let page = list_page(bio, "target.json", "targets", params).await?;
    let targets: Vec<Value> = page.records.iter().map(target_record).collect();
    Ok(json!({
        "source": "ChEMBL",
        "returned": targets.len(),
        "total": page.total_json(),
        "has_more": page.has_more,
        "targets": targets
    }))
}

struct Page {
    records: Vec<Value>,
    total: Option<u64>,
    has_more: bool,
}

impl Page {
    fn total_json(&self) -> Value {
        self.total.map(|total| json!(total)).unwrap_or(Value::Null)
    }
}

fn data_root(bio: &NativeBio) -> String {
    bio.credential("CHEMBL_BASE")
        .unwrap_or(DATA)
        .trim_end_matches('/')
        .to_string()
}

async fn list_page(
    bio: &NativeBio,
    path: &str,
    collection: &str,
    mut params: Vec<(String, String)>,
) -> Result<Page> {
    if !params.iter().any(|(key, _)| key == "format") {
        params.push(("format".into(), "json".into()));
    }
    let url = format!("{}/{path}", data_root(bio));
    let raw = bio
        .http()
        .send(CHEMBL, Method::GET, &url, &params)
        .await?
        .json()?;
    parse_page(&raw, collection)
}

fn parse_page(raw: &Value, collection: &str) -> Result<Page> {
    let object = raw
        .as_object()
        .context("ChEMBL returned an unexpected document")?;
    if object.contains_key("error") || object.contains_key("ERROR") {
        bail!("ChEMBL rejected the request");
    }
    let records = object
        .get(collection)
        .and_then(Value::as_array)
        .context("ChEMBL returned an unexpected document")?;
    let meta = object
        .get("page_meta")
        .and_then(Value::as_object)
        .context("ChEMBL omitted page metadata")?;
    let total = match meta.get("total_count") {
        None | Some(Value::Null) => None,
        Some(value) => Some(as_u64(value).context("ChEMBL returned invalid page metadata")?),
    };
    let next = match meta.get("next") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) if value.is_empty() => None,
        Some(Value::String(value)) => Some(value.as_str()),
        Some(_) => bail!("ChEMBL returned invalid page metadata"),
    };
    let offset = match meta.get("offset") {
        None | Some(Value::Null) => 0,
        Some(value) => as_u64(value).unwrap_or(0),
    };
    let returned = records.len() as u64;
    let has_more = next.is_some() || total.is_some_and(|count| count > offset + returned);
    if let Some(count) = total {
        if count > 0 && records.is_empty() && next.is_none() {
            bail!("ChEMBL returned invalid page metadata");
        }
    }
    Ok(Page {
        records: records.clone(),
        total,
        has_more,
    })
}

fn list_params(limit: u32, order_by: Option<&str>) -> Vec<(String, String)> {
    let mut params = vec![("limit".into(), limit.max(1).to_string())];
    if let Some(order_by) = order_by {
        params.push(("order_by".into(), order_by.into()));
    }
    params
}

fn mechanism_params(
    limit: u32,
    molecule: Option<&str>,
    target: Option<&str>,
    action: Option<&str>,
    parent: bool,
) -> Vec<(String, String)> {
    let mut params = list_params(limit, Some("mec_id"));
    if let Some(id) = molecule {
        let key = if parent {
            "parent_molecule_chembl_id"
        } else {
            "molecule_chembl_id"
        };
        params.push((key.into(), id.to_string()));
    }
    if let Some(id) = target {
        params.push(("target_chembl_id".into(), id.to_string()));
    }
    if let Some(action) = action {
        params.push(("action_type".into(), action.to_string()));
    }
    params
}

fn apply_indication_phase(
    params: &mut Vec<(String, String)>,
    only_approved: bool,
    max_phase: Option<u8>,
) {
    if only_approved {
        params.push(("max_phase_for_ind".into(), "4".into()));
    } else if let Some(phase) = max_phase {
        params.push(("max_phase_for_ind__gte".into(), phase.to_string()));
    }
}

fn molecule_record(molecule: &Value) -> Value {
    let id = text(&molecule["molecule_chembl_id"]).unwrap_or_default();
    let structures = molecule
        .get("molecule_structures")
        .and_then(Value::as_object);
    let properties = molecule
        .get("molecule_properties")
        .filter(|value| !value.is_null())
        .cloned();
    let synonyms = molecule
        .get("molecule_synonyms")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| text(&row["molecule_synonym"]))
                .take(MAX_SYNONYMS)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "molecule_chembl_id": id,
        "pref_name": molecule.get("pref_name").cloned().unwrap_or(Value::Null),
        "molecule_type": molecule.get("molecule_type").cloned().unwrap_or(Value::Null),
        "max_phase": json_num(&molecule["max_phase"]),
        "first_approval": json_num(&molecule["first_approval"]),
        "withdrawn": json_flag(&molecule["withdrawn_flag"]),
        "black_box_warning": json_flag(&molecule["black_box_warning"]),
        "therapeutic_flag": json_flag(&molecule["therapeutic_flag"]),
        "oral": json_flag(&molecule["oral"]),
        "smiles": structures.and_then(|row| row.get("canonical_smiles")).cloned().unwrap_or(Value::Null),
        "inchi_key": structures.and_then(|row| row.get("standard_inchi_key")).cloned().unwrap_or(Value::Null),
        "similarity": json_num(&molecule["similarity"]),
        "properties": properties.as_ref().map(property_block),
        "synonyms": synonyms,
        "url": compound_url(&id)
    })
}

fn property_block(properties: &Value) -> Value {
    json!({
        "full_mwt": json_num(&properties["full_mwt"]),
        "mw_freebase": json_num(&properties["mw_freebase"]),
        "alogp": json_num(&properties["alogp"]),
        "psa": json_num(&properties["psa"]),
        "hba": json_num(&properties["hba"]),
        "hbd": json_num(&properties["hbd"]),
        "rtb": json_num(&properties["rtb"]),
        "aromatic_rings": json_num(&properties["aromatic_rings"]),
        "heavy_atoms": json_num(&properties["heavy_atoms"]),
        "num_ro5_violations": json_num(&properties["num_ro5_violations"]),
        "qed_weighted": json_num(&properties["qed_weighted"]),
        "formula": properties.get("full_molformula").cloned().unwrap_or(Value::Null)
    })
}

fn admet_properties(molecule: &Value) -> Value {
    let properties = molecule
        .get("molecule_properties")
        .cloned()
        .unwrap_or(Value::Null);
    let mut block = property_block(&properties);
    if let Some(object) = block.as_object_mut() {
        object.insert(
            "molecule_chembl_id".into(),
            json!(text(&molecule["molecule_chembl_id"]).unwrap_or_default()),
        );
    }
    block
}

fn activity_record(activity: &Value) -> Value {
    let molecule = text(&activity["molecule_chembl_id"]);
    let target = text(&activity["target_chembl_id"]);
    let assay = text(&activity["assay_chembl_id"]);
    json!({
        "activity_id": json_num(&activity["activity_id"]),
        "molecule_chembl_id": molecule,
        "target_chembl_id": target,
        "target_pref_name": activity.get("target_pref_name").cloned().unwrap_or(Value::Null),
        "target_organism": activity.get("target_organism").cloned().unwrap_or(Value::Null),
        "standard_type": activity.get("standard_type").cloned().unwrap_or(Value::Null),
        "standard_relation": activity.get("standard_relation").cloned().unwrap_or(Value::Null),
        "standard_value": json_num(&activity["standard_value"]),
        "standard_units": activity.get("standard_units").cloned().unwrap_or(Value::Null),
        "pchembl_value": json_num(&activity["pchembl_value"]),
        "assay_chembl_id": assay,
        "assay_type": activity.get("assay_type").cloned().unwrap_or(Value::Null),
        "assay_description": activity.get("assay_description").cloned().unwrap_or(Value::Null),
        "document_chembl_id": activity.get("document_chembl_id").cloned().unwrap_or(Value::Null),
        "molecule_url": molecule.as_deref().map(compound_url),
        "target_url": target.as_deref().map(target_url),
        "assay_url": assay.as_deref().map(assay_url)
    })
}

fn mechanism_record(mechanism: &Value) -> Value {
    let molecule = text(&mechanism["molecule_chembl_id"]);
    let target = text(&mechanism["target_chembl_id"]);
    json!({
        "mec_id": json_num(&mechanism["mec_id"]),
        "molecule_chembl_id": molecule,
        "parent_molecule_chembl_id": mechanism.get("parent_molecule_chembl_id").cloned().unwrap_or(Value::Null),
        "target_chembl_id": target,
        "mechanism_of_action": mechanism.get("mechanism_of_action").cloned().unwrap_or(Value::Null),
        "action_type": mechanism.get("action_type").cloned().unwrap_or(Value::Null),
        "direct_interaction": json_flag(&mechanism["direct_interaction"]),
        "disease_efficacy": json_flag(&mechanism["disease_efficacy"]),
        "max_phase": json_num(&mechanism["max_phase"]),
        "molecule_url": molecule.as_deref().map(compound_url),
        "target_url": target.as_deref().map(target_url)
    })
}

fn target_record(target: &Value) -> Value {
    let id = text(&target["target_chembl_id"]).unwrap_or_default();
    let components = target
        .get("target_components")
        .and_then(Value::as_array)
        .map(|rows| rows.iter().take(MAX_COMPONENTS).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut gene_symbols = Vec::new();
    let mut seen_genes = BTreeSet::new();
    let mut accessions = Vec::new();
    let mut seen_accessions = BTreeSet::new();
    for component in &components {
        if let Some(accession) = text(&component["accession"]) {
            if seen_accessions.insert(accession.clone()) {
                accessions.push(accession);
            }
        }
        if let Some(rows) = component
            .get("target_component_synonyms")
            .and_then(Value::as_array)
        {
            for synonym in rows {
                if text(&synonym["syn_type"]).as_deref() == Some("GENE_SYMBOL") {
                    if let Some(symbol) = text(&synonym["component_synonym"]) {
                        if seen_genes.insert(symbol.clone()) {
                            gene_symbols.push(symbol);
                        }
                    }
                }
            }
        }
    }
    json!({
        "target_chembl_id": id,
        "pref_name": target.get("pref_name").cloned().unwrap_or(Value::Null),
        "target_type": target.get("target_type").cloned().unwrap_or(Value::Null),
        "organism": target.get("organism").cloned().unwrap_or(Value::Null),
        "tax_id": json_num(&target["tax_id"]),
        "gene_symbols": gene_symbols,
        "accessions": accessions,
        "url": target_url(&id)
    })
}

fn warning_record(warning: &Value) -> Value {
    json!({
        "warning_type": warning.get("warning_type").cloned().unwrap_or(Value::Null),
        "warning_class": warning.get("warning_class").cloned().unwrap_or(Value::Null),
        "warning_country": warning.get("warning_country").cloned().unwrap_or(Value::Null),
        "warning_year": json_num(&warning["warning_year"])
    })
}

fn unique_warnings(warnings: Vec<Value>) -> Vec<Value> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for warning in warnings {
        let key = format!(
            "{}|{}|{}|{}",
            warning["warning_type"].as_str().unwrap_or(""),
            warning["warning_class"].as_str().unwrap_or(""),
            warning["warning_country"].as_str().unwrap_or(""),
            warning["warning_year"]
        );
        if seen.insert(key) {
            out.push(warning);
        }
    }
    out
}

fn unique_strings<'a>(rows: impl Iterator<Item = &'a Value>, field: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for row in rows {
        if let Some(value) = text(&row[field]) {
            if seen.insert(value.clone()) {
                out.push(value);
            }
        }
    }
    out
}

fn unique_ids(ids: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for id in ids {
        if seen.insert(id.clone()) {
            out.push(id);
        }
    }
    out
}

fn compound_url(id: &str) -> String {
    format!("{WEB}/compound_report_card/{id}/")
}

fn target_url(id: &str) -> String {
    format!("{WEB}/target_report_card/{id}/")
}

fn assay_url(id: &str) -> String {
    format!("{WEB}/assay_report_card/{id}/")
}

fn bound_limit(limit: u32) -> Result<u32> {
    if !(1..=MAX_LIMIT).contains(&limit) {
        bail!("limit must be between 1 and {MAX_LIMIT}");
    }
    Ok(limit)
}

fn bound_phase(phase: Option<u8>) -> Result<Option<u8>> {
    match phase {
        None => Ok(None),
        Some(value) if value <= 4 => Ok(Some(value)),
        Some(_) => bail!("max_phase must be between 0 and 4"),
    }
}

fn parse_chembl_id(value: &str) -> Result<String> {
    let upper = value.trim().to_ascii_uppercase();
    let digits = upper.strip_prefix("CHEMBL").unwrap_or("");
    if upper.len() < 7
        || upper.len() > 20
        || digits.is_empty()
        || !digits.chars().all(|c| c.is_ascii_digit())
        || digits.starts_with('0')
    {
        bail!("ChEMBL identifier must look like CHEMBL25");
    }
    Ok(upper)
}

fn optional_chembl_id(value: Option<&str>) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(value) if value.trim().is_empty() => Ok(None),
        Some(value) => parse_chembl_id(value).map(Some),
    }
}

fn require_query<'a>(label: &str, value: &'a str, max: usize) -> Result<&'a str> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > max {
        bail!("{label} must contain 1 to {max} characters");
    }
    if trimmed.chars().any(|c| c.is_control()) {
        bail!("{label} must not contain control characters");
    }
    Ok(trimmed)
}

fn optional_query<'a>(label: &str, value: Option<&'a str>, max: usize) -> Result<Option<&'a str>> {
    match value {
        None => Ok(None),
        Some(value) if value.trim().is_empty() => Ok(None),
        Some(value) => require_query(label, value, max).map(Some),
    }
}

fn optional_smiles(value: Option<&str>) -> Result<Option<&str>> {
    match value {
        None => Ok(None),
        Some(value) if value.trim().is_empty() => Ok(None),
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.len() > 2048 {
                bail!("smiles must contain 1 to 2048 characters");
            }
            if trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
                bail!("smiles must not contain whitespace or control characters");
            }
            Ok(Some(trimmed))
        }
    }
}

fn path_segment(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        .filter(|value| value.is_finite())
}

fn as_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|n| u64::try_from(n).ok()))
        .or_else(|| {
            number(value).and_then(|n| {
                if n >= 0.0 && n.fract() == 0.0 && n <= u64::MAX as f64 {
                    Some(n as u64)
                } else {
                    None
                }
            })
        })
}

fn json_num(value: &Value) -> Value {
    match number(value) {
        Some(n) if n.fract() == 0.0 && (i64::MIN as f64..=i64::MAX as f64).contains(&n) => {
            json!(n as i64)
        }
        Some(n) => json!(n),
        None if value.is_null() => Value::Null,
        None => value.clone(),
    }
}

fn json_flag(value: &Value) -> Value {
    match value {
        Value::Null => Value::Null,
        Value::Bool(flag) => json!(flag),
        Value::Number(_) => json!(number(value).is_some_and(|n| n != 0.0)),
        Value::String(text) => match text.as_str() {
            "0" | "false" | "False" | "N" | "n" => json!(false),
            "1" | "true" | "True" | "Y" | "y" => json!(true),
            _ => value.clone(),
        },
        _ => value.clone(),
    }
}

fn cmp_f64_desc(left: Option<f64>, right: Option<f64>) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) => right
            .partial_cmp(&left)
            .unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

fn number_param(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests;
