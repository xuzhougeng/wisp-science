use super::{
    api_base, json_string, path_segment, require_ok, require_text, send_json, text_field,
    unique_ids, ALPHAFOLD, ALPHAFOLD_DEFAULT, ALPHAFOLD_SITE,
};
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};

const MAX_IDS: usize = 40;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetPrediction {
    uniprot_accession: String,
    #[serde(default)]
    include_sequence: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckCoverage {
    uniprot_accessions: Vec<String>,
}

pub(crate) fn fold_uniprot(raw: &str) -> Result<Option<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > 32
        || !trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        bail!("UniProt accession {trimmed:?} is not a valid identifier");
    }
    Ok(Some(trimmed.to_ascii_uppercase()))
}

pub(crate) async fn get_prediction(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetPrediction =
        serde_json::from_value(args.clone()).context("invalid AlphaFold prediction arguments")?;
    let accession = require_text(&args.uniprot_accession, "uniprot_accession", 32)?;
    let accession = fold_uniprot(&accession)?.context("uniprot_accession is required")?;
    match fetch_models(bio, &accession).await? {
        Models::Missing => Ok(json!({
            "source": "AlphaFold DB",
            "source_url": ALPHAFOLD_SITE,
            "uniprot_accession": accession,
            "url": entry_url(&accession),
            "has_model": false,
            "n_models": 0,
            "models": []
        })),
        Models::Invalid(message) => Ok(json!({
            "source": "AlphaFold DB",
            "source_url": ALPHAFOLD_SITE,
            "uniprot_accession": accession,
            "url": entry_url(&accession),
            "has_model": false,
            "n_models": 0,
            "models": [],
            "error": format!("invalid_accession: {message}")
        })),
        Models::Found(models) => {
            let parsed: Vec<Value> = models
                .iter()
                .map(|model| parse_model(model, args.include_sequence))
                .collect();
            Ok(json!({
                "source": "AlphaFold DB",
                "source_url": ALPHAFOLD_SITE,
                "uniprot_accession": accession,
                "url": entry_url(&accession),
                "has_model": !parsed.is_empty(),
                "n_models": parsed.len(),
                "models": parsed
            }))
        }
    }
}

pub(crate) async fn check_coverage(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: CheckCoverage =
        serde_json::from_value(args.clone()).context("invalid AlphaFold coverage arguments")?;
    let ids = unique_ids(
        &args.uniprot_accessions,
        MAX_IDS,
        "UniProt accession",
        fold_uniprot,
    )?;
    let mut records = Vec::new();
    for accession in &ids.unique {
        let record = match fetch_models(bio, accession).await? {
            Models::Missing => json!({
                "uniprot_accession": accession,
                "url": entry_url(accession),
                "has_model": false,
                "n_models": 0
            }),
            Models::Invalid(message) => json!({
                "uniprot_accession": accession,
                "url": entry_url(accession),
                "has_model": false,
                "n_models": 0,
                "error": format!("invalid_accession: {message}")
            }),
            Models::Found(models) => {
                let primary = models.first().cloned().unwrap_or(json!({}));
                json!({
                    "uniprot_accession": accession,
                    "url": entry_url(accession),
                    "has_model": !models.is_empty(),
                    "n_models": models.len(),
                    "model_entity_id": json_string(
                        primary.get("modelEntityId").or(primary.get("entryId"))
                    ),
                    "latest_version": primary.get("latestVersion").cloned().unwrap_or(Value::Null),
                    "global_plddt": primary.get("globalMetricValue").cloned().unwrap_or(Value::Null),
                    "sequence_length": sequence_length(&primary)
                })
            }
        };
        records.push(record);
    }
    Ok(json!({
        "source": "AlphaFold DB",
        "source_url": ALPHAFOLD_SITE,
        "n_requested": ids.requested,
        "n_unique": ids.unique.len(),
        "n_blank_skipped": ids.n_blank,
        "n_duplicate_skipped": ids.n_duplicate,
        "records": records
    }))
}

enum Models {
    Found(Vec<Value>),
    Missing,
    Invalid(String),
}

async fn fetch_models(bio: &NativeBio, accession: &str) -> Result<Models> {
    let base = api_base(bio, "ALPHAFOLD_URL", ALPHAFOLD_DEFAULT);
    let url = format!("{base}/prediction/{}", path_segment(accession));
    let (status, body) = send_json(bio, ALPHAFOLD, Method::GET, &url, &[]).await?;
    if status == StatusCode::NOT_FOUND {
        return Ok(Models::Missing);
    }
    if status == StatusCode::BAD_REQUEST {
        return Ok(Models::Invalid(
            "AlphaFold DB rejected the identifier".into(),
        ));
    }
    require_ok(ALPHAFOLD, status)?;
    let Some(Value::Array(models)) = body else {
        bail!("AlphaFold DB returned a non-list prediction payload");
    };
    if models.is_empty() {
        return Ok(Models::Missing);
    }
    Ok(Models::Found(models))
}

fn parse_model(raw: &Value, include_sequence: bool) -> Value {
    let mut urls = serde_json::Map::new();
    for (key, field) in [
        ("cif", "cifUrl"),
        ("bcif", "bcifUrl"),
        ("pdb", "pdbUrl"),
        ("pae_image", "paeImageUrl"),
        ("pae_json", "paeDocUrl"),
        ("plddt_json", "plddtDocUrl"),
        ("msa", "msaUrl"),
        ("alphamissense_csv", "amAnnotationsUrl"),
    ] {
        if let Some(url) = text_field(raw, &[field]) {
            urls.insert(key.to_string(), json!(url));
        }
    }
    let mut record = json!({
        "model_entity_id": json_string(raw.get("modelEntityId").or(raw.get("entryId"))),
        "entry_id": json_string(raw.get("entryId").or(raw.get("modelEntityId"))),
        "provider_id": json_string(raw.get("providerId")),
        "tool_used": json_string(raw.get("toolUsed")),
        "uniprot_accession": json_string(raw.get("uniprotAccession")),
        "uniprot_id": json_string(raw.get("uniprotId")),
        "uniprot_description": json_string(raw.get("uniprotDescription")),
        "gene": json_string(raw.get("gene")),
        "organism_scientific_name": json_string(raw.get("organismScientificName")),
        "tax_id": raw.get("taxId").cloned().unwrap_or(Value::Null),
        "is_uniprot_reviewed": raw.get("isUniProtReviewed").or(raw.get("isReviewed")).cloned().unwrap_or(Value::Null),
        "is_reference_proteome": raw.get("isUniProtReferenceProteome").or(raw.get("isReferenceProteome")).cloned().unwrap_or(Value::Null),
        "is_complex": raw.get("isComplex").cloned().unwrap_or(Value::Null),
        "sequence_length": sequence_length(raw),
        "uniprot_start": raw.get("sequenceStart").or(raw.get("uniprotStart")).cloned().unwrap_or(Value::Null),
        "uniprot_end": raw.get("sequenceEnd").or(raw.get("uniprotEnd")).cloned().unwrap_or(Value::Null),
        "global_plddt": raw.get("globalMetricValue").cloned().unwrap_or(Value::Null),
        "fraction_plddt": {
            "very_low": raw.get("fractionPlddtVeryLow").cloned().unwrap_or(Value::Null),
            "low": raw.get("fractionPlddtLow").cloned().unwrap_or(Value::Null),
            "confident": raw.get("fractionPlddtConfident").cloned().unwrap_or(Value::Null),
            "very_high": raw.get("fractionPlddtVeryHigh").cloned().unwrap_or(Value::Null)
        },
        "latest_version": raw.get("latestVersion").cloned().unwrap_or(Value::Null),
        "all_versions": raw.get("allVersions").cloned().unwrap_or_else(|| json!([])),
        "model_created_date": json_string(raw.get("modelCreatedDate")),
        "urls": urls
    });
    if include_sequence {
        record["sequence"] = json_string(raw.get("sequence").or(raw.get("uniprotSequence")));
    }
    record
}

fn sequence_length(raw: &Value) -> Value {
    raw.get("sequence")
        .or(raw.get("uniprotSequence"))
        .and_then(Value::as_str)
        .map(|seq| json!(seq.len()))
        .unwrap_or(Value::Null)
}

fn entry_url(accession: &str) -> String {
    format!("{ALPHAFOLD_SITE}/entry/{}", path_segment(accession))
}
