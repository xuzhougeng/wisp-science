//! Open Targets Platform GraphQL (POST /api/v4/graphql).
use super::{
    bound_page, graphql, graphql_data, node, open_targets_endpoint, require_text, tool, MAX_QUERY,
    MAX_VARIABLES, OPEN_TARGETS, OPEN_TARGETS_GRAPHQL, OT_MAX_PAGE,
};
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use wisp_llm::ToolSchema;

const DISEASE_DRUGS_Q: &str = r#"query DiseaseDrugs($id: String!) {
  disease(efoId: $id) {
    id
    name
    drugAndClinicalCandidates {
      count
      rows { id maxClinicalStage drug { id name drugType } }
    }
  }
}"#;

const DISEASE_TARGETS_Q: &str = r#"query DiseaseTargets($id: String!, $size: Int!) {
  disease(efoId: $id) {
    id
    name
    associatedTargets(page: { size: $size, index: 0 }) {
      count
      rows { score target { id approvedSymbol approvedName } }
    }
  }
}"#;

const DRUG_Q: &str = r#"query DrugDetails($id: String!) {
  drug(chemblId: $id) {
    id
    name
    drugType
    maximumClinicalStage
    mechanismsOfAction {
      rows { mechanismOfAction actionType targets { id approvedSymbol } }
    }
  }
}"#;

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        tool(
            "open_targets_disease_drugs",
            "Known and investigational drugs for a disease from the Open Targets Platform GraphQL API (Disease.drugAndClinicalCandidates). efo_id is an EFO/MONDO/Orphanet identifier as used by Open Targets (underscore or colon). size caps the returned rows; count is the upstream total. A missing disease is reported with found=false.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["efo_id"],
                "properties": {
                    "efo_id": {"type": "string", "minLength": 3, "maxLength": 64},
                    "size": {"type": "integer", "minimum": 1, "maximum": 100, "default": 25}
                }
            }),
        ),
        tool(
            "open_targets_disease_targets",
            "Top associated targets for a disease ranked by Open Targets overall association score (Disease.associatedTargets, page index 0). efo_id is an EFO/MONDO identifier. size is the page size (1–100). A missing disease is reported with found=false.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["efo_id"],
                "properties": {
                    "efo_id": {"type": "string", "minLength": 3, "maxLength": 64},
                    "size": {"type": "integer", "minimum": 1, "maximum": 100, "default": 25}
                }
            }),
        ),
        tool(
            "open_targets_drug",
            "Drug annotation from the Open Targets Platform by ChEMBL identifier (drug(chemblId)), including drug type, maximum clinical stage and mechanisms of action. A missing compound is reported with found=false.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["chembl_id"],
                "properties": {
                    "chembl_id": {"type": "string", "minLength": 7, "maxLength": 32, "pattern": "^CHEMBL[0-9]+$"}
                }
            }),
        ),
        tool(
            "open_targets_graphql",
            "POST a read-only GraphQL query to the Open Targets Platform API (https://api.platform.opentargets.org/api/v4/graphql). Provide query and optional variables. Mutations are rejected. GraphQL errors are returned in errors rather than treated as empty evidence. Prefer the typed disease/drug tools when they cover the question.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["query"],
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 8192},
                    "variables": {"type": "object"}
                }
            }),
        ),
    ]
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "open_targets_disease_drugs" => disease_drugs(bio, args).await,
        "open_targets_disease_targets" => disease_targets(bio, args).await,
        "open_targets_drug" => drug(bio, args).await,
        "open_targets_graphql" => passthrough(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiseaseArgs {
    efo_id: String,
    #[serde(default = "super::default_page")]
    size: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DrugArgs {
    chembl_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphqlArgs {
    query: String,
    variables: Option<Value>,
}

async fn disease_drugs(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: DiseaseArgs = serde_json::from_value(args.clone())
        .context("invalid Open Targets disease-drug arguments")?;
    let efo = efo_id(&args.efo_id)?;
    let size = bound_page(args.size, OT_MAX_PAGE)?;
    let payload = ot_graphql(bio, DISEASE_DRUGS_Q, json!({"id": efo}), true).await?;
    let Some(mut disease) = node(&payload, "disease", "Open Targets")? else {
        return Ok(not_found("efo_id", &efo));
    };
    if let Some(rows) = disease
        .pointer_mut("/drugAndClinicalCandidates/rows")
        .and_then(Value::as_array_mut)
    {
        if rows.len() > size as usize {
            rows.truncate(size as usize);
        }
    }
    Ok(wrap_found(
        "efo_id",
        &efo,
        disease,
        json!({"efo_id": efo, "size": size}),
    ))
}

async fn disease_targets(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: DiseaseArgs = serde_json::from_value(args.clone())
        .context("invalid Open Targets disease-target arguments")?;
    let efo = efo_id(&args.efo_id)?;
    let size = bound_page(args.size, OT_MAX_PAGE)?;
    let payload = ot_graphql(
        bio,
        DISEASE_TARGETS_Q,
        json!({"id": efo, "size": size}),
        true,
    )
    .await?;
    let Some(disease) = node(&payload, "disease", "Open Targets")? else {
        return Ok(not_found("efo_id", &efo));
    };
    Ok(wrap_found(
        "efo_id",
        &efo,
        disease,
        json!({"efo_id": efo, "size": size}),
    ))
}

async fn drug(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: DrugArgs =
        serde_json::from_value(args.clone()).context("invalid Open Targets drug arguments")?;
    let chembl = chembl_id(&args.chembl_id)?;
    let payload = ot_graphql(bio, DRUG_Q, json!({"id": chembl}), true).await?;
    let Some(drug) = node(&payload, "drug", "Open Targets")? else {
        return Ok(not_found("chembl_id", &chembl));
    };
    Ok(wrap_found(
        "chembl_id",
        &chembl,
        drug,
        json!({"chembl_id": chembl}),
    ))
}

async fn passthrough(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GraphqlArgs =
        serde_json::from_value(args.clone()).context("invalid Open Targets GraphQL arguments")?;
    let query = require_text(&args.query, "query", MAX_QUERY)?;
    let trimmed = query.trim_start();
    if trimmed.to_ascii_lowercase().starts_with("mutation") {
        bail!("open_targets_graphql is read-only; mutations are not allowed");
    }
    let variables = args.variables.unwrap_or_else(|| json!({}));
    if !variables.is_object() {
        bail!("variables must be a JSON object");
    }
    let encoded = serde_json::to_vec(&variables).context("invalid GraphQL variables")?;
    if encoded.len() > MAX_VARIABLES {
        bail!("variables exceed {MAX_VARIABLES} bytes");
    }
    let payload = ot_graphql(bio, &query, variables, false).await?;
    let mut out = json!({
        "source": "Open Targets Platform",
        "source_url": OPEN_TARGETS_GRAPHQL,
        "data": graphql_data(&payload, "Open Targets")?.clone()
    });
    if let Some(errors) = payload.get("errors") {
        out["errors"] = errors.clone();
    }
    Ok(out)
}

async fn ot_graphql(
    bio: &NativeBio,
    query: &str,
    variables: Value,
    fail_on_errors: bool,
) -> Result<Value> {
    graphql(
        bio,
        OPEN_TARGETS,
        &open_targets_endpoint(bio),
        query,
        variables,
        None,
        fail_on_errors,
    )
    .await
}

fn efo_id(value: &str) -> Result<String> {
    let text = require_text(value, "efo_id", 64)?;
    let normalized = text.replace(':', "_");
    let (prefix, rest) = normalized
        .split_once('_')
        .ok_or_else(|| anyhow_efo(&text))?;
    if prefix.is_empty()
        || !prefix.bytes().all(|b| b.is_ascii_alphabetic())
        || rest.is_empty()
        || !rest.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(anyhow_efo(&text));
    }
    Ok(normalized)
}

fn anyhow_efo(value: &str) -> anyhow::Error {
    anyhow::anyhow!("efo_id {value:?} must look like EFO_0000618, MONDO_0004992 or Orphanet_242")
}

fn chembl_id(value: &str) -> Result<String> {
    let text = require_text(value, "chembl_id", 32)?;
    if !text.starts_with("CHEMBL")
        || !text[6..].bytes().all(|b| b.is_ascii_digit())
        || text.len() < 7
    {
        bail!("chembl_id must look like CHEMBL followed by digits");
    }
    Ok(text)
}

fn wrap_found(key: &str, id: &str, record: Value, query: Value) -> Value {
    let mut out = json!({
        "source": "Open Targets Platform",
        "source_url": OPEN_TARGETS_GRAPHQL,
        "query": query,
        "found": true,
        "record": record
    });
    out[key] = json!(id);
    out
}

fn not_found(key: &str, id: &str) -> Value {
    let mut out = json!({
        "source": "Open Targets Platform",
        "source_url": OPEN_TARGETS_GRAPHQL,
        "found": false,
        "record": Value::Null
    });
    out["query"] = json!({});
    out["query"][key] = json!(id);
    out[key] = json!(id);
    out
}
