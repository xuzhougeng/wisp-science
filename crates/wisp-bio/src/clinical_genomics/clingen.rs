//! ClinGen gene-disease validity, dosage sensitivity, actionability and ERepo.
use super::{
    bound_page, clingen_actionability, clingen_erepo, clingen_search, get_json, json_u64, page,
    require_symbol, require_text, tool, CLINGEN, CLINGEN_ACTIONABILITY, CLINGEN_EREPO,
    CLINGEN_MAX_PAGE, CLINGEN_SEARCH, MAX_TEXT,
};
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use wisp_llm::ToolSchema;

const DOSAGE_LABELS: &[(&str, &str)] = &[
    ("0", "No Evidence"),
    ("1", "Little Evidence"),
    ("2", "Emerging Evidence"),
    ("3", "Sufficient Evidence"),
    ("30", "Gene Associated with Autosomal Recessive Phenotype"),
    ("40", "Dosage Sensitivity Unlikely"),
    ("-5", "Not yet evaluated"),
];

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        tool(
            "clingen_actionability",
            "ClinGen clinical actionability summaries from the Adult and Pediatric JSON APIs (flavor=flat). For a gene, reports intervention/outcome pairs with severity, likelihood, effectiveness and nature-of-intervention scores. context is adult, pediatric or both. The response is a bounded page per context; total_count is the matching row count after gene filtering.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "gene": {"type": "string", "minLength": 1, "maxLength": 64},
                    "context": {"type": "string", "enum": ["adult", "pediatric", "both"], "default": "both"},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 200, "default": 25}
                }
            }),
        ),
        tool(
            "clingen_dosage_sensitivity",
            "ClinGen dosage sensitivity curations (haploinsufficiency and triplosensitivity) from search.clinicalgenome.org/api/dosage. gene is an HGNC symbol or ISCA region id. include_regions adds ISCA genomic-region records; the default listing is genes only. Scores use ClinGen's 0/1/2/3/30/40 scale. Bounded page; total_count is the match count after filtering.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "gene": {"type": "string", "minLength": 1, "maxLength": 64},
                    "include_regions": {"type": "boolean", "default": false},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 200, "default": 25}
                }
            }),
        ),
        tool(
            "clingen_gene_validity",
            "ClinGen gene-disease validity classifications from search.clinicalgenome.org/api/validity (Definitive, Strong, Moderate, Limited, Disputed, Refuted, No Known Disease Relationship). gene is an HGNC symbol matched exactly, case-insensitively. Bounded page; total_count is the match count after filtering.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "gene": {"type": "string", "minLength": 1, "maxLength": 64},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 200, "default": 25}
                }
            }),
        ),
        tool(
            "clingen_variant_classifications",
            "ClinGen Evidence Repository VCEP variant pathogenicity classifications. Provide exactly one of gene (HGNC symbol), caid (ClinGen allele id, e.g. CA114360) or hgvs. Returns a bounded page of interpretation ids, CAIDs, HGVS, conditions and guideline outcomes.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "gene": {"type": "string", "minLength": 1, "maxLength": 64},
                    "caid": {"type": "string", "minLength": 3, "maxLength": 32, "pattern": "^CA[0-9]+$"},
                    "hgvs": {"type": "string", "minLength": 4, "maxLength": 256},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 200, "default": 25}
                }
            }),
        ),
    ]
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "clingen_actionability" => actionability(bio, args).await,
        "clingen_dosage_sensitivity" => dosage(bio, args).await,
        "clingen_gene_validity" => validity(bio, args).await,
        "clingen_variant_classifications" => classifications(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidityQuery {
    gene: Option<String>,
    #[serde(default = "super::default_page")]
    max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DosageQuery {
    gene: Option<String>,
    #[serde(default)]
    include_regions: bool,
    #[serde(default = "super::default_page")]
    max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionabilityQuery {
    gene: Option<String>,
    #[serde(default = "default_context")]
    context: String,
    #[serde(default = "super::default_page")]
    max_results: u32,
}

fn default_context() -> String {
    "both".into()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassificationQuery {
    gene: Option<String>,
    caid: Option<String>,
    hgvs: Option<String>,
    #[serde(default = "super::default_page")]
    max_results: u32,
}

async fn validity(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ValidityQuery =
        serde_json::from_value(args.clone()).context("invalid ClinGen validity arguments")?;
    let cap = bound_page(args.max_results, CLINGEN_MAX_PAGE)?;
    let gene = args
        .gene
        .as_deref()
        .map(|g| require_symbol(g, "gene"))
        .transpose()?;
    let url = format!("{}/api/validity", clingen_search(bio));
    let raw = get_json(bio, CLINGEN, &url, &[]).await?;
    let rows = table_rows(&raw, "validity")?;
    let mut records: Vec<Value> = rows.iter().filter_map(validity_record).collect();
    if let Some(gene) = gene.as_deref() {
        records.retain(|record| {
            record
                .get("gene_symbol")
                .and_then(Value::as_str)
                .is_some_and(|symbol| symbol.eq_ignore_ascii_case(gene))
        });
    }
    records.sort_by(|a, b| {
        rec_str(a, "gene_symbol")
            .cmp(&rec_str(b, "gene_symbol"))
            .then(rec_str(a, "assertion_id").cmp(&rec_str(b, "assertion_id")))
    });
    let total = records.len() as u64;
    Ok(page(
        "ClinGen Gene-Disease Validity",
        &format!("{CLINGEN_SEARCH}/api/validity"),
        json!({"gene": gene, "max_results": cap}),
        records,
        total,
        cap,
        false,
    ))
}

async fn dosage(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: DosageQuery =
        serde_json::from_value(args.clone()).context("invalid ClinGen dosage arguments")?;
    let cap = bound_page(args.max_results, CLINGEN_MAX_PAGE)?;
    let gene = args
        .gene
        .as_deref()
        .map(|g| require_symbol(g, "gene"))
        .transpose()?;
    let url = format!("{}/api/dosage", clingen_search(bio));
    let raw = get_json(bio, CLINGEN, &url, &[]).await?;
    let rows = table_rows(&raw, "dosage")?;
    let mut records: Vec<Value> = rows.iter().filter_map(dosage_record).collect();
    if gene.is_none() && !args.include_regions {
        records.retain(|record| record.get("record_type").and_then(Value::as_str) == Some("gene"));
    }
    if let Some(gene) = gene.as_deref() {
        records.retain(|record| {
            record
                .get("symbol")
                .and_then(Value::as_str)
                .is_some_and(|symbol| symbol.eq_ignore_ascii_case(gene))
                || record
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id.eq_ignore_ascii_case(gene))
        });
    }
    records.sort_by(|a, b| {
        rec_str(a, "record_type")
            .cmp(&rec_str(b, "record_type"))
            .then(rec_str(a, "symbol").cmp(&rec_str(b, "symbol")))
            .then(rec_str(a, "id").cmp(&rec_str(b, "id")))
    });
    let total = records.len() as u64;
    Ok(page(
        "ClinGen Dosage Sensitivity",
        &format!("{CLINGEN_SEARCH}/api/dosage"),
        json!({
            "gene": gene,
            "include_regions": args.include_regions,
            "max_results": cap
        }),
        records,
        total,
        cap,
        false,
    ))
}

async fn actionability(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ActionabilityQuery =
        serde_json::from_value(args.clone()).context("invalid ClinGen actionability arguments")?;
    let cap = bound_page(args.max_results, CLINGEN_MAX_PAGE)?;
    let gene = args
        .gene
        .as_deref()
        .map(|g| require_symbol(g, "gene"))
        .transpose()?;
    let context = args.context.trim().to_ascii_lowercase();
    let contexts: &[&str] = match context.as_str() {
        "adult" => &["Adult"],
        "pediatric" => &["Pediatric"],
        "both" => &["Adult", "Pediatric"],
        _ => bail!("context must be adult, pediatric or both"),
    };
    let mut out = json!({
        "source": "ClinGen Clinical Actionability",
        "source_url": format!("{CLINGEN_ACTIONABILITY}/ac/Adult/api/summ?flavor=flat"),
        "query": {"gene": gene, "context": context, "max_results": cap}
    });
    for ctx in contexts {
        let url = format!("{}/ac/{ctx}/api/summ", clingen_actionability(bio));
        let raw = get_json(bio, CLINGEN, &url, &[("flavor".into(), "flat".into())]).await?;
        let mut records = actionability_records(&raw)?;
        if let Some(gene) = gene.as_deref() {
            records.retain(|record| {
                record
                    .get("genes")
                    .and_then(Value::as_array)
                    .is_some_and(|genes| {
                        genes.iter().any(|item| {
                            item.as_str()
                                .is_some_and(|symbol| symbol.eq_ignore_ascii_case(gene))
                        })
                    })
            });
        }
        records.sort_by(|a, b| {
            rec_str(a, "doc_id")
                .cmp(&rec_str(b, "doc_id"))
                .then(rec_str(a, "outcome").cmp(&rec_str(b, "outcome")))
                .then(rec_str(a, "intervention").cmp(&rec_str(b, "intervention")))
        });
        let total = records.len() as u64;
        let page = page(
            "ClinGen Clinical Actionability",
            &format!("{CLINGEN_ACTIONABILITY}/ac/{ctx}/api/summ?flavor=flat"),
            json!({"gene": gene, "context": ctx.to_ascii_lowercase(), "max_results": cap}),
            records,
            total,
            cap,
            false,
        );
        out[ctx.to_ascii_lowercase()] = page;
    }
    Ok(out)
}

async fn classifications(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ClassificationQuery = serde_json::from_value(args.clone())
        .context("invalid ClinGen variant classification arguments")?;
    let cap = bound_page(args.max_results, CLINGEN_MAX_PAGE)?;
    let mut given = Vec::new();
    if let Some(gene) = args.gene.as_deref() {
        given.push(("gene", require_symbol(gene, "gene")?));
    }
    if let Some(caid) = args.caid.as_deref() {
        let caid = require_text(caid, "caid", 32)?;
        if !caid.starts_with("CA")
            || !caid[2..].bytes().all(|b| b.is_ascii_digit())
            || caid.len() < 3
        {
            bail!("caid must look like CA followed by digits");
        }
        given.push(("caid", caid));
    }
    if let Some(hgvs) = args.hgvs.as_deref() {
        given.push(("hgvs", require_text(hgvs, "hgvs", MAX_TEXT)?));
    }
    if given.len() != 1 {
        bail!("provide exactly one of gene, caid or hgvs");
    }
    let (param, value) = given.into_iter().next().unwrap();
    let url = format!("{}/classifications", clingen_erepo(bio));
    let raw = get_json(
        bio,
        CLINGEN,
        &url,
        &[
            (param.into(), value.clone()),
            ("matchMode".into(), "exact".into()),
            ("matchLimit".into(), cap.to_string()),
        ],
    )
    .await?;
    let interps = raw
        .get("variantInterpretations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut records: Vec<Value> = interps.iter().filter_map(erepo_record).collect();
    records.sort_by(|a, b| rec_str(a, "interpretation_id").cmp(&rec_str(b, "interpretation_id")));
    let total = json_u64(raw.get("total"))
        .or_else(|| json_u64(raw.get("count")))
        .unwrap_or(records.len() as u64);
    let mut query = json!({"max_results": cap});
    query[param] = json!(value);
    let returned = records.len() as u64;
    let total = total.max(returned);
    let has_more = returned >= u64::from(cap) && total > u64::from(cap);
    Ok(page(
        "ClinGen Evidence Repository",
        &format!("{CLINGEN_EREPO}/classifications"),
        query,
        records,
        total,
        cap,
        has_more,
    ))
}

fn table_rows<'a>(raw: &'a Value, what: &str) -> Result<&'a Vec<Value>> {
    let rows = raw
        .get("rows")
        .and_then(Value::as_array)
        .with_context(|| format!("ClinGen omitted {what} rows"))?;
    if let Some(total) = json_u64(raw.get("total")) {
        if total != rows.len() as u64 {
            bail!("ClinGen {what} total did not match the returned row count");
        }
    }
    Ok(rows)
}

fn validity_record(row: &Value) -> Option<Value> {
    let symbol = row.get("symbol").and_then(Value::as_str)?.trim();
    if symbol.is_empty() {
        return None;
    }
    let assertion_id = stringish(row.get("perm_id"));
    let url = assertion_id.as_deref().map(|id| {
        if id.starts_with("http") {
            id.to_string()
        } else {
            format!("{CLINGEN_SEARCH}/kb/gene-validity/{id}")
        }
    });
    Some(json!({
        "gene_symbol": symbol,
        "hgnc_id": stringish(row.get("hgnc_id")),
        "disease_label": stringish(row.get("disease_name")).unwrap_or_default(),
        "mondo_id": stringish(row.get("mondo")),
        "moi": stringish(row.get("moi")),
        "sop": stringish(row.get("sop")),
        "classification": stringish(row.get("classification")).unwrap_or_default(),
        "expert_panel": stringish(row.get("ep")).unwrap_or_default(),
        "affiliate_id": stringish(row.get("affiliate_id")),
        "animal_model_only": row.get("animal_model_only").and_then(Value::as_bool).unwrap_or(false),
        "assertion_id": assertion_id,
        "url": url
    }))
}

fn dosage_record(row: &Value) -> Option<Value> {
    let is_region = match row.get("type") {
        Some(Value::Number(n)) => n.as_i64() == Some(1),
        Some(Value::String(s)) => s.eq_ignore_ascii_case("region") || s == "1",
        _ => false,
    };
    let symbol = stringish(row.get("symbol")).unwrap_or_default();
    Some(json!({
        "record_type": if is_region { "region" } else { "gene" },
        "symbol": symbol,
        "id": stringish(row.get("hgnc_id")),
        "cytoband": stringish(row.get("location")),
        "grch37": row.get("grch37").cloned().unwrap_or(Value::Null),
        "grch38": row.get("grch38").cloned().unwrap_or(Value::Null),
        "haploinsufficiency": dosage_assertion(row.get("haplo_assertion")),
        "triplosensitivity": dosage_assertion(row.get("triplo_assertion")),
        "haplo_disease": stringish(row.get("haplo_disease")),
        "haplo_mondo": stringish(row.get("haplo_mondo")),
        "triplo_disease": stringish(row.get("triplo_disease")),
        "triplo_mondo": stringish(row.get("triplo_mondo")),
        "omim": stringish(row.get("omim")),
        "url": format!("{CLINGEN_SEARCH}/kb/gene-dosage")
    }))
}

fn dosage_assertion(value: Option<&Value>) -> Value {
    let raw = match value {
        None | Some(Value::Null) => return Value::Null,
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.trim().to_string(),
        Some(_) => return Value::Null,
    };
    if raw.is_empty() {
        return Value::Null;
    }
    let code = if raw.eq_ignore_ascii_case("Not yet evaluated") {
        "-5".to_string()
    } else {
        raw.split(':').next().unwrap_or(&raw).trim().to_string()
    };
    let label = DOSAGE_LABELS
        .iter()
        .find(|(item, _)| *item == code)
        .map(|(_, label)| *label)
        .unwrap_or(raw.as_str());
    json!({"code": code, "label": label})
}

fn actionability_records(raw: &Value) -> Result<Vec<Value>> {
    let columns = raw
        .get("columns")
        .and_then(Value::as_array)
        .context("ClinGen omitted actionability columns")?;
    let rows = raw
        .get("rows")
        .and_then(Value::as_array)
        .context("ClinGen omitted actionability rows")?;
    let names: Vec<String> = columns
        .iter()
        .map(|col| col.as_str().unwrap_or("").to_string())
        .collect();
    let mut records = Vec::new();
    for row in rows {
        let cells = match row {
            Value::Array(cells) => cells.clone(),
            Value::Object(map) => names
                .iter()
                .map(|name| map.get(name).cloned().unwrap_or(Value::Null))
                .collect(),
            _ => continue,
        };
        let mut fields = serde_json::Map::new();
        for (name, cell) in names.iter().zip(cells.into_iter()) {
            fields.insert(name.clone(), cell);
        }
        let obj = Value::Object(fields);
        let genes = stringish(obj.get("geneOrVariant"))
            .unwrap_or_default()
            .split(',')
            .map(|g| g.trim().to_string())
            .filter(|g| !g.is_empty())
            .collect::<Vec<_>>();
        records.push(json!({
            "doc_id": stringish(obj.get("docId")),
            "curation_type": stringish(obj.get("curationType")),
            "context": stringish(obj.get("context")),
            "release": stringish(obj.get("release")),
            "release_date": stringish(obj.get("releaseDate")),
            "genes": genes,
            "gene_omim": stringish(obj.get("geneOmim")),
            "disease": stringish(obj.get("disease")),
            "disease_omim": stringish(obj.get("omim")),
            "status_overall": stringish(obj.get("status-overall")),
            "outcome": stringish(obj.get("outcome")),
            "intervention": stringish(obj.get("intervention")),
            "severity": obj.get("severity").cloned(),
            "likelihood": obj.get("likelihood").cloned(),
            "nature_of_intervention": obj.get("natureOfIntervention").cloned(),
            "effectiveness": obj.get("effectiveness").cloned(),
            "overall_score": obj.get("overall").cloned(),
            "url": format!("{CLINGEN_ACTIONABILITY}/ac/")
        }));
    }
    Ok(records)
}

fn erepo_record(interp: &Value) -> Option<Value> {
    if !interp.is_object() {
        return None;
    }
    let gene = interp.get("gene").cloned().unwrap_or(Value::Null);
    let cond = interp.get("condition").cloned().unwrap_or(Value::Null);
    let mut hgvs = interp
        .get("hgvs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    hgvs.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    let guidelines = interp
        .get("guidelines")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|g| {
            json!({
                "guideline": stringish(g.get("label")),
                "guideline_id": stringish(g.get("@id")),
                "outcome": stringish(
                    g.get("outcome")
                        .and_then(|o| o.get("label"))
                        .or_else(|| g.get("outcome"))
                )
            })
        })
        .collect::<Vec<_>>();
    Some(json!({
        "interpretation_id": stringish(interp.get("@id")),
        "uuid": stringish(interp.get("uuid")),
        "caid": stringish(interp.get("caid")),
        "clinvar_variation_id": stringish(interp.get("variationId")),
        "gene_symbol": stringish(gene.get("label")),
        "gene_ncbi_id": stringish(gene.get("NCBI_id")),
        "condition_id": stringish(cond.get("@id")),
        "condition_label": stringish(cond.get("label")),
        "hgvs": hgvs,
        "published_date": stringish(interp.get("publishedDate")),
        "guidelines": guidelines,
        "url": stringish(interp.get("@id"))
    }))
}

fn stringish(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

fn rec_str<'a>(record: &'a Value, key: &str) -> String {
    record
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}
