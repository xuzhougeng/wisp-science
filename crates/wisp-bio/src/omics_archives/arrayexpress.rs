use super::*;
use anyhow::Context;
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Accession {
    accession: String,
    #[serde(default = "default_rows")]
    max_rows_returned: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: Option<String>,
    organism: Option<String>,
    study_type: Option<String>,
    technology: Option<String>,
    released_after: Option<String>,
    released_before: Option<String>,
    #[serde(default)]
    extra_facets: BTreeMap<String, String>,
    #[serde(default = "default_page")]
    max_records: u32,
}

pub(super) async fn search_experiments(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Search =
        serde_json::from_value(args.clone()).context("invalid ArrayExpress search arguments")?;
    let cap = bound_page(args.max_records)?;
    let params = search_params(&args)?;
    let base = api_base(bio, "ARRAYEXPRESS_BASE_URL", BIOSTUDIES);
    let mut records = Vec::new();
    let mut total_hits = None;
    let mut exact = None;
    let mut truncated = false;
    let mut page = 1u32;
    loop {
        let mut page_params = params.clone();
        page_params.push(("pageSize".into(), PAGE_SIZE.to_string()));
        page_params.push(("page".into(), page.to_string()));
        let raw = get_json(
            bio,
            ARRAYEXPRESS,
            &format!("{base}/arrayexpress/search"),
            &page_params,
        )
        .await?;
        if total_hits.is_none() {
            total_hits = field_u64(&raw, &["totalHits", "total_hits"]);
            exact = raw
                .get("isTotalHitsExact")
                .and_then(as_bool)
                .or_else(|| raw.get("is_total_exact").and_then(as_bool));
        }
        let hits = raw
            .get("hits")
            .and_then(Value::as_array)
            .or_else(|| raw.get("studies").and_then(Value::as_array))
            .cloned()
            .unwrap_or_default();
        let batch_len = hits.len();
        for hit in hits {
            if let Some(record) = project_hit(&hit) {
                records.push(record);
            }
            if records.len() >= cap {
                truncated = true;
                break;
            }
        }
        if truncated || batch_len < PAGE_SIZE as usize {
            break;
        }
        if let Some(total) = total_hits {
            if records.len() as u64 >= total {
                break;
            }
        }
        page += 1;
        if page > 5 {
            truncated = true;
            break;
        }
    }
    if !truncated {
        if let Some(total) = total_hits {
            truncated = (records.len() as u64) < total;
        }
    }
    Ok(json!({
        "source": "ArrayExpress (BioStudies)",
        "source_url": "https://www.ebi.ac.uk/biostudies/arrayexpress",
        "query": params_object(&params),
        "total_hits": total_hits,
        "is_total_exact": exact,
        "returned": records.len(),
        "truncated": truncated,
        "records": records,
    }))
}

pub(super) async fn get_experiment(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Accession =
        serde_json::from_value(args.clone()).context("invalid ArrayExpress accession arguments")?;
    let accession = require_ae(&args.accession)?;
    let study = fetch_study(bio, &accession).await?;
    let mut record = flatten_study(&study)?;
    record["source"] = json!("ArrayExpress (BioStudies)");
    record["url"] = json!(ae_url(&accession));
    Ok(record)
}

pub(super) async fn get_experiment_files(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Accession =
        serde_json::from_value(args.clone()).context("invalid ArrayExpress accession arguments")?;
    let accession = require_ae(&args.accession)?;
    let study = fetch_study(bio, &accession).await?;
    let files = collect_files(study.get("section").unwrap_or(&Value::Null));
    let info = fetch_info(bio, &accession).await?;
    let mut records = Vec::new();
    for file in files {
        let path = field_text(&file, &["path", "name"]).unwrap_or_default();
        records.push(json!({
            "path": path,
            "size_bytes": field_u64(&file, &["size", "size_bytes"]),
            "type": file_attr(&file, "Type"),
            "format": file_attr(&file, "Format"),
            "description": file_attr(&file, "Description"),
            "download_url": if path.is_empty() {
                Value::Null
            } else {
                json!(format!("{BIOSTUDIES_FILES}/{}/{}", path_seg(&accession), path.split('/').map(path_seg).collect::<Vec<_>>().join("/")))
            }
        }));
    }
    records.sort_by(|a, b| field_text(a, &["path"]).cmp(&field_text(b, &["path"])));
    Ok(json!({
        "source": "ArrayExpress (BioStudies)",
        "source_url": ae_url(&accession),
        "accession": study.get("accno").and_then(as_text).unwrap_or(accession),
        "n_files": records.len(),
        "files": records,
        "info_reported_file_count": info.as_ref().and_then(|info| field_u64(info, &["files", "fileCount"])),
        "http_link": info.as_ref().and_then(|info| field_text(info, &["httpLink", "http_link"])),
        "ftp_link": info.as_ref().and_then(|info| field_text(info, &["ftpLink", "ftp_link"])),
        "rel_path": info.as_ref().and_then(|info| field_text(info, &["relPath", "rel_path"])),
    }))
}

pub(super) async fn get_experiment_samples(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Accession =
        serde_json::from_value(args.clone()).context("invalid ArrayExpress accession arguments")?;
    let accession = require_ae(&args.accession)?;
    let cap = bound_rows(args.max_rows_returned)?;
    let study = fetch_study(bio, &accession).await?;
    let files = collect_files(study.get("section").unwrap_or(&Value::Null));
    let mut sdrf = files
        .into_iter()
        .filter(|file| file_attr(file, "Type").as_deref() == Some("SDRF File"))
        .collect::<Vec<_>>();
    sdrf.sort_by(|a, b| field_text(a, &["path"]).cmp(&field_text(b, &["path"])));
    let Some(sdrf) = sdrf.first() else {
        return Ok(json!({
            "source": "ArrayExpress (BioStudies)",
            "source_url": ae_url(&accession),
            "accession": study.get("accno").and_then(as_text).unwrap_or(accession),
            "error": "no_sdrf",
            "n_samples": 0,
            "n_samples_returned": 0,
            "rows_truncated": false,
            "samples": [],
        }));
    };
    let path = field_text(sdrf, &["path", "name"]).context("SDRF file omitted its path")?;
    let files_base = api_base(bio, "ARRAYEXPRESS_FILES_URL", BIOSTUDIES_FILES);
    let url = format!(
        "{files_base}/{}/{}",
        path_seg(&accession),
        path.split('/').map(path_seg).collect::<Vec<_>>().join("/")
    );
    let text = bio.http().ebi_download(ARRAYEXPRESS, &url).await?.text()?;
    let parsed = parse_sdrf(&text)?;
    let n_samples = parsed.samples.len();
    let truncated = n_samples > cap;
    let rows: Vec<Value> = parsed
        .samples
        .into_iter()
        .take(cap)
        .map(Value::Object)
        .collect();
    Ok(json!({
        "source": "ArrayExpress (BioStudies)",
        "source_url": ae_url(&accession),
        "accession": study.get("accno").and_then(as_text).unwrap_or(accession),
        "sdrf_file": path,
        "headers": parsed.headers,
        "n_samples": n_samples,
        "n_samples_returned": rows.len(),
        "rows_truncated": truncated,
        "samples": rows,
    }))
}

async fn fetch_study(bio: &NativeBio, accession: &str) -> Result<Value> {
    let base = api_base(bio, "ARRAYEXPRESS_BASE_URL", BIOSTUDIES);
    get_json(
        bio,
        ARRAYEXPRESS,
        &format!("{base}/studies/{}", path_seg(accession)),
        &[],
    )
    .await
}

async fn fetch_info(bio: &NativeBio, accession: &str) -> Result<Option<Value>> {
    let base = api_base(bio, "ARRAYEXPRESS_BASE_URL", BIOSTUDIES);
    let response = send(
        bio,
        ARRAYEXPRESS,
        Method::GET,
        &format!("{base}/studies/{}/info", path_seg(accession)),
        &[],
    )
    .await?;
    if missing_status(response.status) {
        return Ok(None);
    }
    Ok(Some(response.json()?))
}

fn search_params(args: &Search) -> Result<Vec<(String, String)>> {
    let mut clauses = Vec::new();
    if let Some(query) = args.query.as_deref() {
        clauses.push(require_text(query, "query", MAX_QUERY)?.to_string());
    }
    match (
        args.released_after.as_deref(),
        args.released_before.as_deref(),
    ) {
        (None, None) => {}
        (after, before) => {
            let lo = after.map(iso_date).transpose()?.unwrap_or("*");
            let hi = before.map(iso_date).transpose()?.unwrap_or("*");
            clauses.push(format!("release_date:[{lo} TO {hi}]"));
        }
    }
    let mut params = Vec::new();
    if !clauses.is_empty() {
        params.push(("query".into(), clauses.join(" AND ")));
    }
    if let Some(organism) = args.organism.as_deref() {
        params.push((
            "facet.organism".into(),
            require_text(organism, "organism", 256)?.to_string(),
        ));
    }
    if let Some(study_type) = args.study_type.as_deref() {
        params.push((
            "facet.study_type".into(),
            require_text(study_type, "study_type", 256)?.to_string(),
        ));
    }
    if let Some(technology) = args.technology.as_deref() {
        params.push((
            "facet.technology".into(),
            require_text(technology, "technology", 256)?.to_string(),
        ));
    }
    for (key, value) in &args.extra_facets {
        let key = key.trim();
        let value = require_text(value, "facet value", 256)?;
        if key.is_empty()
            || key.len() > 64
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
        {
            bail!("facet names must be alphanumeric (optionally with '.' / '_')");
        }
        let name = if key.starts_with("facet.") {
            key.to_string()
        } else {
            format!("facet.{key}")
        };
        params.push((name, value.to_string()));
    }
    if params.is_empty() {
        bail!("provide a query or at least one ArrayExpress filter");
    }
    params.push(("sortBy".into(), "release_date".into()));
    params.push(("sortOrder".into(), "descending".into()));
    Ok(params)
}

fn params_object(params: &[(String, String)]) -> Value {
    let mut map = serde_json::Map::new();
    for (key, value) in params {
        if key == "page" || key == "pageSize" {
            continue;
        }
        map.insert(key.clone(), json!(value));
    }
    Value::Object(map)
}

fn project_hit(hit: &Value) -> Option<Value> {
    let accession = field_text(hit, &["accession", "accno"])?;
    Some(json!({
        "accession": accession,
        "title": field_text(hit, &["title"]),
        "release_date": field_text(hit, &["release_date", "releaseDate"]),
        "files": field_u64(hit, &["files"]),
        "links": field_u64(hit, &["links"]),
        "is_public": hit.get("isPublic").and_then(as_bool).or_else(|| hit.get("is_public").and_then(as_bool)),
        "url": ae_url(&accession),
    }))
}

fn require_ae(value: &str) -> Result<String> {
    let accession = value.trim().to_ascii_uppercase();
    let rest = accession.strip_prefix("E-").unwrap_or("");
    let mut parts = rest.split('-');
    let prefix = parts.next().unwrap_or("");
    let number = parts.next().unwrap_or("");
    if prefix.len() != 4
        || !prefix.bytes().all(|b| b.is_ascii_alphabetic())
        || number.is_empty()
        || !number.bytes().all(|b| b.is_ascii_digit())
        || parts.next().is_some()
    {
        bail!("ArrayExpress accession {value:?} must look like E-MTAB-5061");
    }
    Ok(accession)
}

fn ae_url(accession: &str) -> String {
    format!(
        "https://www.ebi.ac.uk/biostudies/arrayexpress/studies/{}",
        path_seg(accession)
    )
}

pub(super) fn flatten_study(study: &Value) -> Result<Value> {
    let section = study.get("section").unwrap_or(&Value::Null);
    let title = attr_value(section, "Title").or_else(|| top_attr(study, "Title"));
    let mut organisms = attr_values(section, "Organism");
    organisms.sort();
    organisms.dedup();
    let mut sample_count: Option<u64> = None;
    let mut experimental_designs = Vec::new();
    let mut experimental_factors = Vec::new();
    let mut assay_count: Option<u64> = None;
    let mut technology = None;
    let mut assay_by_molecule = None;
    let mut protocol_types = Vec::new();
    let mut protocol_count = 0usize;
    let mut authors = Vec::new();
    let mut org_names = BTreeMap::new();
    let mut publications = Vec::new();
    for node in walk_sections(section) {
        match node.get("type").and_then(Value::as_str) {
            Some("Samples") => {
                if sample_count.is_none() {
                    sample_count = attr_value(node, "Sample count").and_then(|v| v.parse().ok());
                }
                experimental_designs.extend(attr_values(node, "Experimental Designs"));
                experimental_factors.extend(attr_values(node, "Experimental Factors"));
            }
            Some("Assays and Data") => {
                if assay_count.is_none() {
                    assay_count = attr_value(node, "Assay count").and_then(|v| v.parse().ok());
                }
                if technology.is_none() {
                    technology = attr_value(node, "Technology");
                }
                if assay_by_molecule.is_none() {
                    assay_by_molecule = attr_value(node, "Assay by Molecule");
                }
            }
            Some("Protocols") => {
                protocol_count += 1;
                protocol_types.extend(attr_values(node, "Type"));
            }
            Some("Organization" | "Organisation") => {
                if let Some(name) = attr_value(node, "Name") {
                    let key = node.get("accno").and_then(as_text).unwrap_or_default();
                    org_names.insert(key, name);
                }
            }
            Some("Author") => {
                let refs = attr_values(node, "affiliation");
                authors.push(json!({
                    "name": attr_value(node, "Name"),
                    "email": attr_value(node, "Email"),
                    "role": attr_value(node, "Role"),
                    "affiliations": refs.iter().map(|r| org_names.get(r).cloned().unwrap_or_else(|| r.clone())).collect::<Vec<_>>(),
                }));
            }
            Some("Publication") => {
                publications.push(json!({
                    "title": attr_value(node, "Title"),
                    "authors": attr_value(node, "Authors"),
                    "doi": attr_value(node, "DOI"),
                    "status": attr_value(node, "Status"),
                }));
            }
            _ => {}
        }
    }
    experimental_designs.sort();
    experimental_designs.dedup();
    experimental_factors.sort();
    experimental_factors.dedup();
    protocol_types.sort();
    protocol_types.dedup();
    let mut submitter_organizations: Vec<_> = org_names.values().cloned().collect();
    submitter_organizations.sort();
    submitter_organizations.dedup();
    let files = collect_files(section);
    let mut files_by_type = BTreeMap::new();
    let mut total_bytes = 0u64;
    for file in &files {
        let kind = file_attr(file, "Type")
            .or_else(|| file_attr(file, "Description"))
            .unwrap_or_else(|| "unspecified".into());
        *files_by_type.entry(kind).or_insert(0u64) += 1;
        if let Some(size) = field_u64(file, &["size", "size_bytes"]) {
            total_bytes += size;
        }
    }
    let mut links = Vec::new();
    for link in collect_links(section) {
        if let Some(target) = field_text(&link, &["url", "target"]) {
            links.push(json!({
                "target": target,
                "type": file_attr(&link, "Type"),
            }));
        }
    }
    let accession = study
        .get("accno")
        .and_then(as_text)
        .context("ArrayExpress study omitted its accession")?;
    Ok(json!({
        "accession": accession,
        "title": title,
        "release_date": top_attr(study, "ReleaseDate"),
        "study_type": attr_value(section, "Study type"),
        "organisms": organisms,
        "description": attr_value(section, "Description"),
        "assay_count": assay_count,
        "sample_count": sample_count,
        "technology": technology,
        "assay_by_molecule": assay_by_molecule,
        "experimental_designs": experimental_designs,
        "experimental_factors": experimental_factors,
        "authors": authors,
        "submitter_organizations": submitter_organizations,
        "publications": publications,
        "protocol_count": protocol_count,
        "protocol_types": protocol_types,
        "file_count": files.len(),
        "files_by_type": files_by_type,
        "total_file_bytes": total_bytes,
        "links": links,
    }))
}

fn walk_sections(node: &Value) -> Vec<&Value> {
    let mut out = Vec::new();
    walk_sections_into(node, &mut out);
    out
}

fn walk_sections_into<'a>(node: &'a Value, out: &mut Vec<&'a Value>) {
    match node {
        Value::Array(items) => {
            for item in items {
                walk_sections_into(item, out);
            }
        }
        Value::Object(_) => {
            out.push(node);
            if let Some(subs) = node.get("subsections") {
                walk_sections_into(subs, out);
            }
        }
        _ => {}
    }
}

fn collect_files(section: &Value) -> Vec<Value> {
    let mut files = Vec::new();
    for node in walk_sections(section) {
        push_entries(node.get("files"), &mut files);
    }
    files
}

fn collect_links(section: &Value) -> Vec<Value> {
    let mut links = Vec::new();
    for node in walk_sections(section) {
        push_entries(node.get("links"), &mut links);
    }
    links
}

fn push_entries(value: Option<&Value>, out: &mut Vec<Value>) {
    match value {
        Some(Value::Array(items)) => {
            for item in items {
                if item.is_array() {
                    push_entries(Some(item), out);
                } else if item.is_object() {
                    out.push(item.clone());
                }
            }
        }
        Some(item) if item.is_object() => out.push(item.clone()),
        _ => {}
    }
}

fn attr_values(node: &Value, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(Value::Array(attrs)) = node.get("attributes") {
        for attr in attrs {
            if attr.get("name").and_then(Value::as_str) == Some(name) {
                if let Some(value) = attr.get("value").and_then(as_text) {
                    out.push(value);
                }
            }
        }
    }
    out
}

fn attr_value(node: &Value, name: &str) -> Option<String> {
    attr_values(node, name).into_iter().next()
}

fn top_attr(study: &Value, name: &str) -> Option<String> {
    attr_value(study, name)
}

fn file_attr(file: &Value, name: &str) -> Option<String> {
    attr_value(file, name)
}

pub(super) struct Sdrf {
    pub headers: Vec<String>,
    pub samples: Vec<serde_json::Map<String, Value>>,
}

pub(super) fn parse_sdrf(text: &str) -> Result<Sdrf> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in text.lines() {
        if line.split('\t').all(|cell| cell.trim().is_empty()) {
            continue;
        }
        rows.push(line.split('\t').map(|cell| cell.to_string()).collect());
    }
    if rows.is_empty() {
        return Ok(Sdrf {
            headers: Vec::new(),
            samples: Vec::new(),
        });
    }
    let headers = dedupe_headers(&rows[0]);
    let mut samples = Vec::new();
    for (i, row) in rows.iter().skip(1).enumerate() {
        if row.len() > headers.len() {
            bail!(
                "SDRF line {} has {} fields but the header has {} — refusing to truncate",
                i + 2,
                row.len(),
                headers.len()
            );
        }
        let mut map = serde_json::Map::new();
        for (idx, header) in headers.iter().enumerate() {
            map.insert(
                header.clone(),
                json!(row.get(idx).cloned().unwrap_or_default()),
            );
        }
        samples.push(map);
    }
    Ok(Sdrf { headers, samples })
}

fn dedupe_headers(headers: &[String]) -> Vec<String> {
    let mut seen = BTreeMap::new();
    let mut out = Vec::new();
    for header in headers {
        let name = header.trim();
        let n = seen.entry(name.to_string()).or_insert(0);
        *n += 1;
        if *n == 1 {
            out.push(name.to_string());
        } else {
            out.push(format!("{name}#{n}"));
        }
    }
    out
}
