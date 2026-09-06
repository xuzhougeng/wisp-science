use super::{
    api_base, as_bool, as_f64, as_i64, as_u64, bound_int, json_string, listify, object_field,
    path_segment, require_ok, require_text, send_json, text_field, unique_ids, unwrap_value, EMDB,
    EMDB_DEFAULT, EMDB_SITE,
};
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};

const MAX_IDS: usize = 25;
const PAGE_ROWS: usize = 50;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetEntries {
    emdb_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetSection {
    emdb_ids: Vec<String>,
    section: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: String,
    #[serde(default = "default_max_rows")]
    max_rows: u32,
}

fn default_max_rows() -> u32 {
    100
}

pub(crate) fn fold_emdb_id(raw: &str) -> Result<Option<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let upper = trimmed.to_ascii_uppercase();
    let digits = upper.strip_prefix("EMD-").unwrap_or(&upper);
    if !digits.bytes().all(|b| b.is_ascii_digit()) || digits.is_empty() || digits.len() > 12 {
        bail!("EMDB accession {trimmed:?} is not EMD-n, emd-n or a numeric accession");
    }
    Ok(Some(format!("EMD-{digits}")))
}

pub(crate) async fn get_entries(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetEntries =
        serde_json::from_value(args.clone()).context("invalid EMDB entry arguments")?;
    let ids = unique_ids(&args.emdb_ids, MAX_IDS, "EMDB accession", fold_emdb_id)?;
    let mut records = Vec::new();
    for emdb_id in &ids.unique {
        records.push(match fetch_entry(bio, emdb_id).await? {
            None => json!({"emdb_id": emdb_id, "error": "not_found", "url": entry_url(emdb_id)}),
            Some(raw) => extract_entry(&raw),
        });
    }
    Ok(json!({
        "source": "EMDB",
        "source_url": EMDB_SITE,
        "n_requested": ids.requested,
        "n_unique": ids.unique.len(),
        "n_blank_skipped": ids.n_blank,
        "n_duplicate_skipped": ids.n_duplicate,
        "records": records
    }))
}

pub(crate) async fn get_entry_section(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetSection =
        serde_json::from_value(args.clone()).context("invalid EMDB section arguments")?;
    let section = args.section.trim();
    if !matches!(section, "publications" | "map" | "sample" | "imaging") {
        bail!("section must be publications, map, sample or imaging");
    }
    let ids = unique_ids(&args.emdb_ids, MAX_IDS, "EMDB accession", fold_emdb_id)?;
    let mut records = Vec::new();
    for emdb_id in &ids.unique {
        records.push(match fetch_entry(bio, emdb_id).await? {
            None => json!({"emdb_id": emdb_id, "error": "not_found", "url": entry_url(emdb_id)}),
            Some(raw) => match section {
                "publications" => extract_publications(&raw),
                "map" => extract_map(&raw),
                "sample" => extract_sample(&raw),
                _ => extract_imaging(&raw),
            },
        });
    }
    Ok(json!({
        "source": "EMDB",
        "source_url": EMDB_SITE,
        "section": section,
        "n_requested": ids.requested,
        "n_unique": ids.unique.len(),
        "records": records
    }))
}

pub(crate) async fn get_validation(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetEntries =
        serde_json::from_value(args.clone()).context("invalid EMDB validation arguments")?;
    let ids = unique_ids(&args.emdb_ids, MAX_IDS, "EMDB accession", fold_emdb_id)?;
    let mut records = Vec::new();
    for emdb_id in &ids.unique {
        records.push(fetch_validation(bio, emdb_id).await?);
    }
    Ok(json!({
        "source": "EMDB",
        "source_url": EMDB_SITE,
        "n_requested": ids.requested,
        "n_unique": ids.unique.len(),
        "records": records
    }))
}

pub(crate) async fn search_entries(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Search =
        serde_json::from_value(args.clone()).context("invalid EMDB search arguments")?;
    let query = require_text(&args.query, "query", 512)?;
    let cap = bound_int(args.max_rows, 1, 1000, "max_rows")?;
    let num_found_released = facet_released_count(bio, &query).await?;
    let mut records: Vec<Value> = Vec::new();
    let mut page = 1u32;
    while records.len() < cap {
        let rows = fetch_search_page(bio, &query, page).await?;
        if rows.is_empty() {
            break;
        }
        let exhausted = rows.len() < PAGE_ROWS;
        for row in rows {
            if records.len() >= cap {
                break;
            }
            let compact = compact_search_row(&row);
            let id = compact
                .get("emdb_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if !id.is_empty()
                && records
                    .iter()
                    .any(|r| r.get("emdb_id").and_then(Value::as_str) == Some(id.as_str()))
            {
                continue;
            }
            records.push(compact);
        }
        if exhausted {
            break;
        }
        page += 1;
        if page > 40 {
            break;
        }
    }
    records.sort_by(|a, b| emd_sort_key(a).cmp(&emd_sort_key(b)));
    let mut by_status = serde_json::Map::new();
    for record in &records {
        let status = record
            .get("current_status")
            .and_then(Value::as_str)
            .unwrap_or("UNKNOWN");
        let count = by_status.get(status).and_then(Value::as_u64).unwrap_or(0);
        by_status.insert(status.to_string(), json!(count + 1));
    }
    let released_rows = by_status.get("REL").and_then(Value::as_u64).unwrap_or(0);
    let truncated = match num_found_released {
        Some(total) => (released_rows as usize) < total || records.len() >= cap,
        None => records.len() >= cap,
    };
    Ok(json!({
        "source": "EMDB",
        "source_url": EMDB_SITE,
        "query": query,
        "max_rows": cap,
        "num_found_released": json!(num_found_released),
        "returned": records.len(),
        "rows_by_status": by_status,
        "released_complete": num_found_released.is_some_and(|n| n == released_rows as usize) && !truncated,
        "truncated": truncated,
        "records": records
    }))
}

async fn fetch_entry(bio: &NativeBio, emdb_id: &str) -> Result<Option<Value>> {
    let base = api_base(bio, "EMDB_URL", EMDB_DEFAULT);
    let url = format!("{base}/entry/{}", path_segment(emdb_id));
    let (status, body) = send_json(bio, EMDB, Method::GET, &url, &[]).await?;
    if status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    require_ok(EMDB, status)?;
    Ok(Some(body.context("EMDB returned an empty entry body")?))
}

async fn fetch_validation(bio: &NativeBio, emdb_id: &str) -> Result<Value> {
    let base = api_base(bio, "EMDB_URL", EMDB_DEFAULT);
    let url = format!("{base}/analysis/{}", path_segment(emdb_id));
    let (status, body) = send_json(bio, EMDB, Method::GET, &url, &[]).await?;
    if status == StatusCode::NOT_FOUND {
        return Ok(json!({
            "emdb_id": emdb_id,
            "url": entry_url(emdb_id),
            "has_validation_analysis": false,
            "error": "not_found"
        }));
    }
    require_ok(EMDB, status)?;
    let payload = body.unwrap_or(json!({}));
    Ok(extract_validation(&payload, emdb_id))
}

async fn facet_released_count(bio: &NativeBio, query: &str) -> Result<Option<usize>> {
    let base = api_base(bio, "EMDB_URL", EMDB_DEFAULT);
    let url = format!("{base}/facet/{}", path_segment(query));
    let (status, body) = send_json(
        bio,
        EMDB,
        Method::GET,
        &url,
        &[("field".into(), "current_status".into())],
    )
    .await?;
    if status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    require_ok(EMDB, status)?;
    let Some(body) = body else {
        return Ok(None);
    };
    let counts = body
        .get("current_status")
        .and_then(Value::as_object)
        .or_else(|| body.as_object());
    let Some(counts) = counts else {
        return Ok(None);
    };
    if let Some(rel) = counts.get("REL").and_then(as_u64) {
        return Ok(Some(rel as usize));
    }
    let sum: u64 = counts.values().filter_map(as_u64).sum();
    Ok(Some(sum as usize))
}

async fn fetch_search_page(bio: &NativeBio, query: &str, page: u32) -> Result<Vec<Value>> {
    let base = api_base(bio, "EMDB_URL", EMDB_DEFAULT);
    let url = format!("{base}/search/{}", path_segment(query));
    let (status, body) = send_json(
        bio,
        EMDB,
        Method::GET,
        &url,
        &[
            ("rows".into(), PAGE_ROWS.to_string()),
            ("page".into(), page.to_string()),
            (
                "fl".into(),
                "emdb_id,title,resolution,structure_determination_method,fitted_pdbs,current_status,release_date"
                    .into(),
            ),
        ],
    )
    .await?;
    require_ok(EMDB, status)?;
    Ok(match body {
        None => Vec::new(),
        Some(Value::Array(rows)) => rows,
        Some(Value::Object(map)) => map
            .get("results")
            .or(map.get("entries"))
            .or(map.get("records"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        Some(_) => Vec::new(),
    })
}

fn extract_entry(entry: &Value) -> Value {
    let admin = object_or_empty(entry, "admin");
    let xref = object_or_empty(entry, "crossreferences");
    let sample = object_or_empty(entry, "sample");
    let map = object_or_empty(entry, "map");
    let status_code = admin
        .get("current_status")
        .map(unwrap_value)
        .and_then(|node| text_field(node, &["code"]).or_else(|| scalar_text(unwrap_value(node))));
    let obsolete = object_or_empty(&admin, "obsolete_list");
    let mut superseded = Vec::new();
    for item in listify(obsolete.get("entry")) {
        if let Some(repl) = text_field(item, &["entry"]).or_else(|| scalar_text(item)) {
            superseded.push(repl);
        }
    }
    superseded.sort();
    superseded.dedup();
    let sd_list = object_or_empty(entry, "structure_determination_list");
    let sd = listify(sd_list.get("structure_determination"))
        .into_iter()
        .next()
        .cloned()
        .unwrap_or(json!({}));
    let image_processing = listify(sd.get("image_processing"))
        .into_iter()
        .next()
        .cloned()
        .unwrap_or(json!({}));
    let final_rec = object_or_empty(&image_processing, "final_reconstruction");
    let resolution = final_rec
        .get("resolution")
        .map(unwrap_value)
        .and_then(as_f64)
        .or_else(|| as_f64(final_rec.get("resolution").unwrap_or(&Value::Null)));
    let emdb_id = text_field(entry, &["emdb_id"]).unwrap_or_default();
    let macromolecule_list = object_or_empty(&sample, "macromolecule_list");
    let supramolecule_list = object_or_empty(&sample, "supramolecule_list");
    let macromolecules = named_list(&macromolecule_list, "macromolecule");
    let supramolecules = named_list(&supramolecule_list, "supramolecule");
    let pdb_list = object_or_empty(&xref, "pdb_list");
    let mut fitted = Vec::new();
    for pdb in listify(pdb_list.get("pdb_reference")) {
        if let Some(id) = text_field(pdb, &["pdb_id"]) {
            fitted.push(id.to_ascii_lowercase());
        }
    }
    fitted.sort();
    fitted.dedup();
    let dates = object_or_empty(&admin, "key_dates");
    let pixel = object_or_empty(&map, "pixel_spacing");
    json!({
        "emdb_id": emdb_id,
        "url": entry_url(&emdb_id),
        "title": json_string(admin.get("title")),
        "status": json!(status_code),
        "is_obsolete": status_code.as_deref() == Some("OBS"),
        "superseded_by": superseded,
        "method": json_string(sd.get("method")),
        "aggregation_state": json_string(sd.get("aggregation_state")),
        "resolution_angstrom": json!(resolution),
        "resolution_method": json_string(final_rec.get("resolution_method")),
        "deposition_date": date_only(dates.get("deposition")),
        "header_release_date": date_only(dates.get("header_release")),
        "map_release_date": date_only(dates.get("map_release")),
        "update_date": date_only(dates.get("update")),
        "sample_name": scalar_text(sample.get("name").unwrap_or(&Value::Null)),
        "macromolecule_names": macromolecules,
        "supramolecule_names": supramolecules,
        "fitted_pdb_ids": fitted,
        "has_fitted_model": !fitted.is_empty(),
        "citation": extract_primary_citation(&xref),
        "map": {
            "file": json_string(map.get("file")),
            "size_kbytes": map.get("size_kbytes").cloned().unwrap_or(Value::Null),
            "dimensions": object_or_empty(&map, "dimensions"),
            "voxel_size_angstrom": {
                "x": unit_value(pixel.get("x")),
                "y": unit_value(pixel.get("y")),
                "z": unit_value(pixel.get("z"))
            }
        }
    })
}

fn extract_publications(entry: &Value) -> Value {
    let xref = object_or_empty(entry, "crossreferences");
    let cit_list = object_or_empty(&xref, "citation_list");
    let emdb_id = text_field(entry, &["emdb_id"]).unwrap_or_default();
    let primary = parse_citation_node(cit_list.get("primary_citation"));
    let mut secondary = Vec::new();
    if let Some(map) = cit_list.as_object() {
        for (key, node) in map {
            if key == "primary_citation" {
                continue;
            }
            for item in listify(Some(node)) {
                if let Some(parsed) = parse_citation_node(Some(item)) {
                    if parsed != Value::Null {
                        secondary.push(parsed);
                    }
                }
            }
        }
    }
    json!({
        "emdb_id": emdb_id,
        "url": entry_url(&emdb_id),
        "primary_citation": primary,
        "secondary_citations": secondary
    })
}

fn extract_map(entry: &Value) -> Value {
    let map = object_or_empty(entry, "map");
    let emdb_id = text_field(entry, &["emdb_id"]).unwrap_or_default();
    let contour_list = object_or_empty(&map, "contour_list");
    let contours: Vec<Value> = listify(contour_list.get("contour"))
        .into_iter()
        .map(|c| {
            json!({
                "level": as_f64(c.get("level").unwrap_or(&Value::Null)),
                "primary": as_bool(c.get("primary").unwrap_or(&Value::Null)),
                "source": json_string(c.get("source"))
            })
        })
        .collect();
    let stats = object_or_empty(&map, "statistics");
    let pixel = object_or_empty(&map, "pixel_spacing");
    let cell = object_or_empty(&map, "cell");
    let symmetry = object_or_empty(&map, "symmetry");
    json!({
        "emdb_id": emdb_id,
        "url": entry_url(&emdb_id),
        "file": json_string(map.get("file")),
        "format": json_string(map.get("format")),
        "size_kbytes": map.get("size_kbytes").cloned().unwrap_or(Value::Null),
        "data_type": json_string(map.get("data_type")),
        "dimensions": object_or_empty(&map, "dimensions"),
        "origin": object_or_empty(&map, "origin"),
        "spacing": object_or_empty(&map, "spacing"),
        "axis_order": object_or_empty(&map, "axis_order"),
        "pixel_spacing_angstrom": {
            "x": unit_value(pixel.get("x")),
            "y": unit_value(pixel.get("y")),
            "z": unit_value(pixel.get("z"))
        },
        "cell": {
            "a": unit_value(cell.get("a")),
            "b": unit_value(cell.get("b")),
            "c": unit_value(cell.get("c")),
            "alpha": unit_value(cell.get("alpha")),
            "beta": unit_value(cell.get("beta")),
            "gamma": unit_value(cell.get("gamma"))
        },
        "statistics": {
            "minimum": as_f64(stats.get("minimum").unwrap_or(&Value::Null)),
            "maximum": as_f64(stats.get("maximum").unwrap_or(&Value::Null)),
            "average": as_f64(stats.get("average").unwrap_or(&Value::Null)),
            "std": as_f64(stats.get("std").unwrap_or(&Value::Null))
        },
        "contour_levels": contours,
        "space_group": json!(symmetry.get("space_group").map(unwrap_value).and_then(scalar_text)),
        "label": json_string(map.get("label"))
    })
}

fn extract_sample(entry: &Value) -> Value {
    let sample = object_or_empty(entry, "sample");
    let emdb_id = text_field(entry, &["emdb_id"]).unwrap_or_default();
    let macromolecule_list = object_or_empty(&sample, "macromolecule_list");
    let macromolecules: Vec<Value> = listify(macromolecule_list.get("macromolecule"))
        .into_iter()
        .map(|m| {
            let seq = object_or_empty(m, "sequence");
            let refs: Vec<Value> = listify(seq.get("external_references"))
                .into_iter()
                .map(|r| {
                    json!({
                        "type": json_string(r.get("type_")),
                        "id": scalar_text(r)
                    })
                })
                .collect();
            json!({
                "id": m.get("macromolecule_id").cloned().unwrap_or(Value::Null),
                "type": json_string(m.get("instance_type")),
                "name": scalar_text(m.get("name").unwrap_or(&Value::Null)),
                "molecular_weight": molecular_weight(m.get("molecular_weight")),
                "number_of_copies": m.get("number_of_copies").cloned().unwrap_or(Value::Null),
                "ec_number": listify(m.get("ec_number")).into_iter().filter_map(scalar_text).collect::<Vec<_>>(),
                "natural_source": natural_source(m.get("natural_source")),
                "sequence_external_references": refs
            })
        })
        .collect();
    let supramolecule_list = object_or_empty(&sample, "supramolecule_list");
    let supramolecules: Vec<Value> = listify(supramolecule_list.get("supramolecule"))
        .into_iter()
        .map(|s| {
            json!({
                "id": s.get("supramolecule_id").cloned().unwrap_or(Value::Null),
                "type": json_string(s.get("instance_type")),
                "name": scalar_text(s.get("name").unwrap_or(&Value::Null)),
                "parent": s.get("parent").cloned().unwrap_or(Value::Null),
                "molecular_weight": molecular_weight(s.get("molecular_weight")),
                "natural_source": natural_source(s.get("natural_source"))
            })
        })
        .collect();
    json!({
        "emdb_id": emdb_id,
        "url": entry_url(&emdb_id),
        "name": scalar_text(sample.get("name").unwrap_or(&Value::Null)),
        "macromolecules": macromolecules,
        "supramolecules": supramolecules
    })
}

fn extract_imaging(entry: &Value) -> Value {
    let sd_list = object_or_empty(entry, "structure_determination_list");
    let sd = listify(sd_list.get("structure_determination"))
        .into_iter()
        .next()
        .cloned()
        .unwrap_or(json!({}));
    let emdb_id = text_field(entry, &["emdb_id"]).unwrap_or_default();
    let microscopy_list = object_or_empty(&sd, "microscopy_list");
    let sessions: Vec<Value> = listify(microscopy_list.get("microscopy"))
        .into_iter()
        .map(|mic| {
            let recording_list = object_or_empty(mic, "image_recording_list");
            let recordings: Vec<Value> = listify(recording_list.get("image_recording"))
                .into_iter()
                .map(|rec| {
                    json!({
                        "id": rec.get("image_recording_id").cloned().unwrap_or(Value::Null),
                        "detector": scalar_text(rec.get("film_or_detector_model").unwrap_or(&Value::Null)),
                        "average_electron_dose_per_image": unit_value(rec.get("average_electron_dose_per_image")),
                        "number_real_images": rec.get("number_real_images").cloned().unwrap_or(Value::Null)
                    })
                })
                .collect();
            json!({
                "id": mic.get("microscopy_id").cloned().unwrap_or(Value::Null),
                "type": json_string(mic.get("instance_type")),
                "microscope": json_string(mic.get("microscope")),
                "acceleration_voltage": unit_value(mic.get("acceleration_voltage")),
                "electron_source": json_string(mic.get("electron_source")),
                "illumination_mode": json_string(mic.get("illumination_mode")),
                "imaging_mode": json_string(mic.get("imaging_mode")),
                "nominal_cs": unit_value(mic.get("nominal_cs")),
                "nominal_defocus_min": unit_value(mic.get("nominal_defocus_min")),
                "nominal_defocus_max": unit_value(mic.get("nominal_defocus_max")),
                "image_recordings": recordings
            })
        })
        .collect();
    let prep_list = object_or_empty(&sd, "specimen_preparation_list");
    let preparations: Vec<Value> = listify(prep_list.get("specimen_preparation"))
        .into_iter()
        .map(|prep| {
            let buffer = object_or_empty(prep, "buffer");
            let grid = object_or_empty(prep, "grid");
            let vit = object_or_empty(prep, "vitrification");
            json!({
                "id": prep.get("preparation_id").cloned().unwrap_or(Value::Null),
                "buffer": {"ph": as_f64(buffer.get("ph").unwrap_or(&Value::Null)), "details": json_string(buffer.get("details"))},
                "grid": {
                    "material": json_string(grid.get("material")),
                    "mesh": grid.get("mesh").cloned().unwrap_or(Value::Null),
                    "model": json_string(grid.get("model"))
                },
                "vitrification": {
                    "cryogen_name": json_string(vit.get("cryogen_name")),
                    "instrument": json_string(vit.get("instrument"))
                }
            })
        })
        .collect();
    json!({
        "emdb_id": emdb_id,
        "url": entry_url(&emdb_id),
        "method": json_string(sd.get("method")),
        "microscopy": sessions,
        "specimen_preparations": preparations
    })
}

fn extract_validation(payload: &Value, emdb_id: &str) -> Value {
    let num = emdb_id.rsplit('-').next().unwrap_or(emdb_id);
    let inner = payload
        .get(num)
        .cloned()
        .or_else(|| payload.get(emdb_id).cloned())
        .unwrap_or(json!({}));
    let available = inner
        .as_object()
        .map(|map| map.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let qscore = object_or_empty(&inner, "qscore")
        .get("allmodels_average_qscore")
        .and_then(as_f64);
    let atom = object_or_empty(&inner, "atom_inclusion_by_level")
        .get("average_ai_allmodels")
        .and_then(as_f64);
    let resolution = object_or_empty(&inner, "resolution")
        .get("value")
        .and_then(as_f64);
    json!({
        "emdb_id": emdb_id,
        "url": entry_url(emdb_id),
        "has_validation_analysis": inner.as_object().is_some_and(|m| !m.is_empty()),
        "resolution_angstrom": json!(resolution),
        "qscore_average": json!(qscore),
        "atom_inclusion_average": json!(atom),
        "available_blocks": available,
        "recommended_contour_level": inner.get("recommended_contour_level").cloned().unwrap_or(Value::Null),
        "predicated_contour_level": inner.get("predicated_contour_level").cloned().unwrap_or(Value::Null)
    })
}

fn extract_primary_citation(xref: &Value) -> Value {
    let cit_list = object_or_empty(xref, "citation_list");
    parse_citation_node(cit_list.get("primary_citation")).unwrap_or(Value::Null)
}

fn parse_citation_node(node: Option<&Value>) -> Option<Value> {
    let Value::Object(map) = node? else {
        return None;
    };
    if map.is_empty() {
        return Some(Value::Null);
    }
    let inner = if map.contains_key("external_references") || map.contains_key("author") {
        Value::Object(map.clone())
    } else {
        map.values()
            .find(|v| v.is_object())
            .cloned()
            .unwrap_or_else(|| Value::Object(map.clone()))
    };
    let authors: Vec<Value> = listify(inner.get("author"))
        .into_iter()
        .filter_map(|a| {
            scalar_text(a).map(|name| {
                json!({"name": name, "order": a.get("order").cloned().unwrap_or(Value::Null)})
            })
        })
        .collect();
    let mut doi = Value::Null;
    let mut pmid = Value::Null;
    for refer in listify(inner.get("external_references")) {
        let kind = text_field(refer, &["type_"])
            .unwrap_or_default()
            .to_ascii_uppercase();
        let val = scalar_text(refer).unwrap_or_default();
        if kind == "DOI" && doi.is_null() {
            doi = json!(val.trim_start_matches("doi:").trim_start_matches("DOI:"));
        } else if kind == "PUBMED" && pmid.is_null() {
            pmid = json!(val);
        }
    }
    Some(json!({
        "title": json_string(inner.get("title")),
        "authors": authors,
        "journal": json_string(inner.get("journal_abbreviation").or(inner.get("journal"))),
        "year": inner.get("year").and_then(as_i64),
        "published": as_bool(inner.get("published").unwrap_or(&Value::Null)),
        "first_author": authors.first().and_then(|a| a.get("name")).cloned().unwrap_or(Value::Null),
        "author_count": authors.len(),
        "doi": doi,
        "pmid": pmid,
        "external_references": {"doi": doi, "pmid": pmid}
    }))
}

fn compact_search_row(row: &Value) -> Value {
    if row.get("admin").is_some() {
        let full = extract_entry(row);
        return json!({
            "emdb_id": full.get("emdb_id"),
            "url": full.get("url"),
            "title": full.get("title"),
            "resolution": full.get("resolution_angstrom"),
            "structure_determination_method": full.get("method"),
            "fitted_pdbs": full.get("fitted_pdb_ids"),
            "current_status": full.get("status"),
            "release_date": full.get("map_release_date")
        });
    }
    let emdb_id = text_field(row, &["emdb_id", "emdbId"]).unwrap_or_default();
    json!({
        "emdb_id": emdb_id,
        "url": entry_url(&emdb_id),
        "title": json_string(row.get("title")),
        "resolution": row.get("resolution").cloned().unwrap_or(Value::Null),
        "structure_determination_method": json_string(row.get("structure_determination_method")),
        "fitted_pdbs": row.get("fitted_pdbs").cloned().unwrap_or_else(|| json!([])),
        "current_status": json_string(row.get("current_status")),
        "release_date": json_string(row.get("release_date"))
    })
}

fn object_or_empty(value: &Value, key: &str) -> Value {
    object_field(value, key)
        .map(|map| Value::Object(map.clone()))
        .unwrap_or(json!({}))
}

fn named_list(parent: &Value, key: &str) -> Vec<String> {
    listify(parent.get(key))
        .into_iter()
        .filter_map(|item| scalar_text(item.get("name").unwrap_or(item)))
        .collect()
}

fn scalar_text(node: &Value) -> Option<String> {
    let inner = unwrap_value(node);
    match inner {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Object(map) => map
            .get("value")
            .and_then(scalar_text)
            .or_else(|| text_field(inner, &["code", "entry", "name"])),
        _ => None,
    }
}

fn date_only(value: Option<&Value>) -> Value {
    scalar_text(value.unwrap_or(&Value::Null))
        .map(|text| json!(text.chars().take(10).collect::<String>()))
        .unwrap_or(Value::Null)
}

fn unit_value(node: Option<&Value>) -> Value {
    let Some(node) = node else {
        return json!({"value": Value::Null, "units": Value::Null});
    };
    if let Some(obj) = node.as_object() {
        json!({
            "value": as_f64(obj.get("valueOf_").or(obj.get("value")).unwrap_or(&Value::Null)),
            "units": json_string(obj.get("units"))
        })
    } else {
        json!({"value": as_f64(node), "units": Value::Null})
    }
}

fn molecular_weight(node: Option<&Value>) -> Value {
    let Some(node) = node else {
        return Value::Null;
    };
    json!({
        "theoretical": unit_value(node.get("theoretical")),
        "experimental": unit_value(node.get("experimental"))
    })
}

fn natural_source(node: Option<&Value>) -> Value {
    let Some(node) = node else {
        return Value::Null;
    };
    let organism = node.get("organism").unwrap_or(&Value::Null);
    json!({
        "organism": scalar_text(organism),
        "ncbi_taxid": organism.get("ncbi").cloned().unwrap_or(Value::Null)
    })
}

fn emd_sort_key(row: &Value) -> (u8, u64, String) {
    let id = row
        .get("emdb_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let digits = id.rsplit('-').next().unwrap_or(id);
    match digits.parse::<u64>() {
        Ok(n) => (0, n, id.to_string()),
        Err(_) => (1, 0, id.to_string()),
    }
}

fn entry_url(emdb_id: &str) -> String {
    format!("{EMDB_SITE}/{}", path_segment(emdb_id))
}
