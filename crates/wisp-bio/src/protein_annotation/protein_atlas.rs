use super::{
    bound_page, default_columns, default_max, get_json, hpa_base, path_segment, require_text,
    Fetch, HPA, HPA_SITE, MAX_COLUMNS, MAX_QUERY,
};
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Map, Value};

const RESOLVE_COLUMNS: &str = "g,gs,eg";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GeneArgs {
    pub gene: String,
    #[serde(default)]
    pub full: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchArgs {
    pub query: String,
    #[serde(default = "default_columns")]
    pub columns: String,
    #[serde(default = "default_max")]
    pub max_results: u32,
}

pub(super) async fn get_protein_atlas_gene(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GeneArgs =
        serde_json::from_value(args.clone()).context("invalid Protein Atlas gene arguments")?;
    let gene = require_text(&args.gene, "gene", 64)?;
    let ensembl = if is_ensg(&gene) {
        gene.to_ascii_uppercase()
    } else {
        resolve_symbol(bio, &gene).await?
    };
    let url = format!("{}/{}.json", hpa_base(bio), path_segment(&ensembl));
    let record = match get_json(bio, HPA, &url, &[]).await? {
        Fetch::Json(Value::Object(map)) => Value::Object(map),
        Fetch::Json(_) => bail!("Human Protein Atlas gene JSON was not an object"),
        Fetch::Empty | Fetch::NotFound => {
            bail!("Human Protein Atlas has no gene {ensembl}")
        }
    };
    let symbol = record.get("Gene").and_then(Value::as_str).unwrap_or(&gene);
    let mut out = json!({
        "source": "Human Protein Atlas",
        "source_url": format!("{HPA_SITE}/{ensembl}"),
        "ensembl": ensembl,
        "gene": symbol,
        "full": args.full
    });
    if args.full {
        out["record"] = record;
    } else {
        out["summary"] = summarize(&record);
    }
    Ok(out)
}

pub(super) async fn search_protein_atlas(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchArgs =
        serde_json::from_value(args.clone()).context("invalid Protein Atlas search arguments")?;
    let query = require_text(&args.query, "query", MAX_QUERY)?;
    let columns = parse_columns(&args.columns)?;
    let cap = bound_page(args.max_results)?;
    let rows = search_download(bio, &query, &columns).await?;
    let total = rows.len();
    let records: Vec<Value> = rows.into_iter().take(cap).collect();
    Ok(json!({
        "source": "Human Protein Atlas",
        "source_url": format!("{HPA_SITE}/api/search_download.php"),
        "query": {"query": query, "columns": columns, "max_results": cap},
        "total_available": total,
        "returned": records.len(),
        "has_more": total > records.len(),
        "results": records
    }))
}

async fn resolve_symbol(bio: &NativeBio, symbol: &str) -> Result<String> {
    let rows = search_download(bio, symbol, RESOLVE_COLUMNS).await?;
    let want = symbol.to_ascii_uppercase();
    let mut exact = Vec::new();
    let mut synonym = Vec::new();
    for row in &rows {
        let Some(ensg) = row.get("Ensembl").and_then(Value::as_str) else {
            continue;
        };
        if !is_ensg(ensg) {
            continue;
        }
        let gene = row
            .get("Gene")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_uppercase();
        if gene == want {
            exact.push((
                row.get("Gene")
                    .and_then(Value::as_str)
                    .unwrap_or(ensg)
                    .to_string(),
                ensg.to_string(),
            ));
            continue;
        }
        if synonyms(row.get("Gene synonym"))
            .iter()
            .any(|alias| alias.eq_ignore_ascii_case(symbol))
        {
            synonym.push((
                row.get("Gene")
                    .and_then(Value::as_str)
                    .unwrap_or(ensg)
                    .to_string(),
                ensg.to_string(),
            ));
        }
    }
    let hits = if exact.is_empty() { synonym } else { exact };
    let mut ensgs: Vec<String> = hits.iter().map(|(_, ensg)| ensg.clone()).collect();
    ensgs.sort();
    ensgs.dedup();
    match ensgs.as_slice() {
        [] => bail!("Human Protein Atlas has no gene or synonym {symbol:?}"),
        [one] => Ok(one.clone()),
        _ => bail!(
            "gene symbol {symbol:?} matches multiple Human Protein Atlas genes: {}",
            hits.iter()
                .map(|(name, ensg)| format!("{name} ({ensg})"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

async fn search_download(bio: &NativeBio, query: &str, columns: &str) -> Result<Vec<Value>> {
    let url = format!("{}/api/search_download.php", hpa_base(bio));
    let params = vec![
        ("search".into(), query.to_string()),
        ("format".into(), "json".into()),
        ("columns".into(), columns.to_string()),
        ("compress".into(), "no".into()),
    ];
    match get_json(bio, HPA, &url, &params).await? {
        Fetch::Json(Value::Array(rows)) => Ok(rows),
        Fetch::Json(_) => bail!("Human Protein Atlas search did not return a JSON array"),
        Fetch::Empty => Ok(Vec::new()),
        Fetch::NotFound => bail!("Human Protein Atlas search returned HTTP 404"),
    }
}

fn parse_columns(value: &str) -> Result<String> {
    let mut codes = Vec::new();
    for part in value.split(',') {
        let code = part.trim();
        if code.is_empty() {
            continue;
        }
        if code.len() > 80
            || !code
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '(' | ')'))
        {
            bail!("columns contains an invalid HPA column specifier {code:?}");
        }
        codes.push(code.to_string());
    }
    if codes.is_empty() {
        bail!("columns must list at least one HPA specifier");
    }
    if codes.len() > MAX_COLUMNS {
        bail!("columns lists more than {MAX_COLUMNS} specifiers");
    }
    Ok(codes.join(","))
}

pub(crate) fn is_ensg(value: &str) -> bool {
    let v = value.as_bytes();
    v.len() == 15
        && v[..4].eq_ignore_ascii_case(b"ENSG")
        && v[4..].iter().all(|b| b.is_ascii_digit())
}

fn synonyms(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect(),
        Some(Value::String(text)) => text
            .split([',', ';'])
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn summarize(record: &Value) -> Value {
    let identity = take_keys(
        record,
        &[
            "Gene",
            "Gene synonym",
            "Ensembl",
            "Gene description",
            "Uniprot",
            "Chromosome",
            "Position",
            "Protein class",
            "Biological process",
            "Molecular function",
            "Disease involvement",
            "Evidence",
        ],
    );
    let expression = take_keys(
        record,
        &[
            "RNA tissue specificity",
            "RNA tissue distribution",
            "RNA tissue specific nTPM",
            "Protein tissue specificity",
            "Protein tissue distribution",
            "RNA cancer specificity",
            "RNA blood cell specificity",
            "RNA brain regional specificity",
        ],
    );
    let localization = take_keys(
        record,
        &[
            "Subcellular location",
            "Subcellular main location",
            "Subcellular additional location",
            "Secretome location",
            "Reliability (IF)",
        ],
    );
    let antibodies = take_keys(
        record,
        &[
            "Antibody",
            "Antibody RRID",
            "Reliability (IH)",
            "Reliability (IF)",
        ],
    );
    let mut prognostics = Map::new();
    if let Value::Object(map) = record {
        for (key, value) in map {
            if let Some(cancer) = key.strip_prefix("Cancer prognostics - ") {
                prognostics.insert(cancer.to_string(), value.clone());
            }
        }
    }
    json!({
        "identity": identity,
        "expression": expression,
        "localization": localization,
        "antibodies": antibodies,
        "pathology": {"prognostics": prognostics}
    })
}

fn take_keys(record: &Value, keys: &[&str]) -> Map<String, Value> {
    let mut out = Map::new();
    for key in keys {
        if let Some(value) = record.get(*key) {
            out.insert((*key).to_string(), value.clone());
        }
    }
    out
}
