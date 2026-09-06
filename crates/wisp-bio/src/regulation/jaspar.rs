use super::*;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetMatrix {
    matrix_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MatrixVersions {
    base_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListMatrices {
    collection: Option<String>,
    tax_group: Option<String>,
    tax_id: Option<u64>,
    name: Option<String>,
    search: Option<String>,
    version: Option<String>,
    #[serde(default = "super::default_page")]
    page: u32,
    #[serde(default = "super::default_rows")]
    max_rows: u32,
}

pub async fn get_matrix(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetMatrix =
        serde_json::from_value(args.clone()).context("invalid JASPAR matrix arguments")?;
    let matrix_id = versioned_matrix_id(&args.matrix_id)?;
    let url = join_url(
        &jaspar_base(bio),
        &format!("matrix/{}/", path_segment(&matrix_id)),
    );
    let payload = get_json_ok(bio, JASPAR, &url, &[("format".into(), "json".into())]).await?;
    if payload.get("detail").is_some() && payload.get("matrix_id").is_none() {
        bail!("JASPAR has no matrix {matrix_id}");
    }
    Ok(project_matrix(&payload, &matrix_id))
}

pub async fn matrix_versions(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: MatrixVersions =
        serde_json::from_value(args.clone()).context("invalid JASPAR versions arguments")?;
    let base_id = base_matrix_id(&args.base_id)?;
    let url = join_url(
        &jaspar_base(bio),
        &format!("matrix/{}/versions/", path_segment(&base_id)),
    );
    let payload = get_json_ok(
        bio,
        JASPAR,
        &url,
        &[
            ("format".into(), "json".into()),
            ("page_size".into(), DRF_PAGE_SIZE.to_string()),
        ],
    )
    .await?;
    let (count, results, next) = drf_page(&payload)?;
    let versions: Vec<Value> = results.iter().filter_map(project_matrix_summary).collect();
    if next.is_none() && results.len() as u64 != count {
        bail!(
            "JASPAR versions for {base_id} returned {} rows but count={count}",
            results.len()
        );
    }
    Ok(json!({
        "source": "JASPAR",
        "source_url": format!("https://jaspar.elixir.no/matrix/{base_id}"),
        "base_id": base_id,
        "count": count,
        "returned": versions.len(),
        "truncated": next.is_some() || (versions.len() as u64) < count,
        "versions": versions,
    }))
}

pub async fn list_matrices(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ListMatrices =
        serde_json::from_value(args.clone()).context("invalid JASPAR matrix list arguments")?;
    let page = bound_page(args.page)?;
    let page_size = bound_rows(args.max_rows, LIST_MAX_ROWS)?.min(DRF_PAGE_SIZE);
    if let Some(version) = args
        .version
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        if version != "latest" {
            bail!("version must be latest when set");
        }
    }
    let mut params = vec![
        ("format".into(), "json".into()),
        ("page".into(), page.to_string()),
        ("page_size".into(), page_size.to_string()),
    ];
    let mut query = json!({"page": page, "page_size": page_size});
    if let Some(value) = optional_query(&args.collection, 64, "collection")? {
        query["collection"] = json!(value);
        params.push(("collection".into(), value));
    }
    if let Some(value) = optional_query(&args.tax_group, 64, "tax_group")? {
        query["tax_group"] = json!(value);
        params.push(("tax_group".into(), value));
    }
    if let Some(tax_id) = args.tax_id {
        if tax_id == 0 {
            bail!("tax_id must be a positive NCBI taxonomy id");
        }
        query["tax_id"] = json!(tax_id);
        params.push(("tax_id".into(), tax_id.to_string()));
    }
    if let Some(value) = optional_query(&args.name, 128, "name")? {
        query["name"] = json!(value);
        params.push(("name".into(), value));
    }
    if let Some(value) = optional_query(&args.search, 256, "search")? {
        query["search"] = json!(value);
        params.push(("search".into(), value));
    }
    if let Some(version) = args
        .version
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        query["version"] = json!(version);
        params.push(("version".into(), version.to_string()));
    }
    let url = join_url(&jaspar_base(bio), "matrix/");
    let payload = get_json_ok(bio, JASPAR, &url, &params).await?;
    let (count, results, next) = drf_page(&payload)?;
    let matrices: Vec<Value> = results.iter().filter_map(project_matrix_summary).collect();
    Ok(json!({
        "source": "JASPAR",
        "source_url": "https://jaspar.elixir.no/api/v1/matrix/",
        "query": query,
        "count": count,
        "returned": matrices.len(),
        "truncated": next.is_some() || (matrices.len() as u64) < count,
        "page": page,
        "matrices": matrices,
    }))
}

pub async fn list_species(bio: &NativeBio, args: &Value) -> Result<Value> {
    empty_args(args, "JASPAR species")?;
    catalog(bio, "species/", "species", project_species).await
}

pub async fn list_taxa(bio: &NativeBio, args: &Value) -> Result<Value> {
    empty_args(args, "JASPAR taxa")?;
    catalog(bio, "taxon/", "taxa", project_taxon).await
}

pub async fn list_collections(bio: &NativeBio, args: &Value) -> Result<Value> {
    empty_args(args, "JASPAR collections")?;
    catalog(bio, "collections/", "collections", project_named).await
}

pub async fn list_releases(bio: &NativeBio, args: &Value) -> Result<Value> {
    empty_args(args, "JASPAR releases")?;
    catalog(bio, "releases/", "releases", project_release).await
}

fn empty_args(args: &Value, what: &str) -> Result<()> {
    match args {
        Value::Null => Ok(()),
        Value::Object(map) if map.is_empty() => Ok(()),
        _ => bail!("{what} takes no arguments"),
    }
}

async fn catalog(
    bio: &NativeBio,
    path: &str,
    key: &str,
    project: fn(&Value) -> Option<Value>,
) -> Result<Value> {
    let base = jaspar_base(bio);
    let (count, results, truncated) = drf_catalog(bio, JASPAR, &base, path).await?;
    let rows: Vec<Value> = results.iter().filter_map(project).collect();
    Ok(json!({
        "source": "JASPAR",
        "source_url": format!("https://jaspar.elixir.no/api/v1/{path}"),
        "count": count,
        "returned": rows.len(),
        "truncated": truncated || (rows.len() as u64) < count,
        key: rows,
    }))
}

fn versioned_matrix_id(value: &str) -> Result<String> {
    let id = value.trim().to_ascii_uppercase();
    if !is_matrix_id(&id, true) {
        bail!("matrix_id must be a versioned JASPAR id such as MA0002.2");
    }
    Ok(id)
}

fn base_matrix_id(value: &str) -> Result<String> {
    let id = value.trim().to_ascii_uppercase();
    let base = id.split_once('.').map(|(base, _)| base).unwrap_or(&id);
    if !is_matrix_id(base, false) {
        bail!("base_id must be a JASPAR base id such as MA0002");
    }
    Ok(base.to_string())
}

fn is_matrix_id(value: &str, versioned: bool) -> bool {
    let (base, version) = match value.split_once('.') {
        Some((base, version)) => (base, Some(version)),
        None => (value, None),
    };
    if versioned != version.is_some() {
        return false;
    }
    let base_ok = base.len() == 6
        && base[..2].bytes().all(|b| b.is_ascii_uppercase())
        && base[2..].bytes().all(|b| b.is_ascii_digit());
    let version_ok = version
        .is_none_or(|v| !v.is_empty() && v.len() <= 4 && v.bytes().all(|b| b.is_ascii_digit()));
    base_ok && version_ok
}

fn project_matrix(doc: &Value, matrix_id: &str) -> Value {
    let id = doc
        .get("matrix_id")
        .and_then(Value::as_str)
        .unwrap_or(matrix_id);
    json!({
        "source": "JASPAR",
        "source_url": format!("https://jaspar.elixir.no/matrix/{id}"),
        "api_url": format!("{JASPAR_API}/matrix/{}/", path_segment(id)),
        "matrix_id": id,
        "base_id": doc.get("base_id"),
        "version": doc.get("version"),
        "name": doc.get("name"),
        "collection": doc.get("collection"),
        "tax_group": doc.get("tax_group"),
        "species": doc.get("species"),
        "class": doc.get("class"),
        "family": doc.get("family"),
        "type": doc.get("type"),
        "pfm": doc.get("pfm"),
        "sequence_logo": doc.get("sequence_logo"),
        "pubmed_ids": doc.get("pubmed_ids"),
        "uniprot_ids": doc.get("uniprot_ids"),
        "tfe_id": doc.get("tfe_id"),
        "pazar_tf_id": doc.get("pazar_tf_id"),
        "comment": doc.get("comment"),
        "sites_url": doc.get("sites_url"),
    })
}

fn project_matrix_summary(row: &Value) -> Option<Value> {
    let matrix_id = row.get("matrix_id").and_then(Value::as_str)?;
    Some(json!({
        "matrix_id": matrix_id,
        "name": row.get("name"),
        "collection": row.get("collection"),
        "base_id": row.get("base_id"),
        "version": row.get("version"),
        "sequence_logo": row.get("sequence_logo"),
        "url": row.get("url").cloned().unwrap_or_else(|| {
            json!(format!("{JASPAR_API}/matrix/{}/", path_segment(matrix_id)))
        }),
    }))
}

fn project_species(row: &Value) -> Option<Value> {
    Some(json!({
        "tax_id": row.get("tax_id"),
        "species": row.get("species").cloned().or_else(|| row.get("name").cloned()),
        "url": row.get("url"),
        "matrix_url": row.get("matrix_url"),
    }))
}

fn project_taxon(row: &Value) -> Option<Value> {
    let name = row
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| row.get("tax_group").and_then(Value::as_str))?;
    Some(json!({
        "name": name,
        "url": row.get("url"),
    }))
}

fn project_named(row: &Value) -> Option<Value> {
    let name = row.get("name").and_then(Value::as_str)?;
    Some(json!({
        "name": name,
        "url": row.get("url"),
    }))
}

fn project_release(row: &Value) -> Option<Value> {
    Some(json!({
        "release_number": row.get("release_number").cloned().or_else(|| row.get("release").cloned()),
        "year": row.get("year"),
        "active": row.get("active"),
        "url": row.get("url"),
    }))
}
