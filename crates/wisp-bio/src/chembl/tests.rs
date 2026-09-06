use super::*;
use crate::http::Http;
use crate::NativeBio;
use axum::{
    extract::Query,
    http::{StatusCode, Uri},
    response::IntoResponse,
    Router,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

fn test_bio(base: &str) -> NativeBio {
    NativeBio::test_client(
        &[
            ("CHEMBL_BASE".into(), base.trim_end_matches('/').to_string()),
            ("CHEMBL_API_KEY".into(), "synthetic-key&value".into()),
        ],
        Http(reqwest::Client::builder().no_proxy().build().unwrap()),
    )
    .unwrap()
}

fn empty_page(key: &str) -> Value {
    json!({
        key: [],
        "page_meta": {
            "limit": 20,
            "offset": 0,
            "total_count": 0,
            "next": null,
            "previous": null
        }
    })
}

fn page(key: &str, records: Value, total: u64, next: Option<&str>) -> Value {
    json!({
        key: records,
        "page_meta": {
            "limit": 20,
            "offset": 0,
            "total_count": total,
            "next": next,
            "previous": null
        }
    })
}

fn invented_molecule() -> Value {
    json!({
        "molecule_chembl_id": "CHEMBL9990001",
        "pref_name": "SYNTHALIN",
        "molecule_type": "Small molecule",
        "max_phase": "4.0",
        "first_approval": 1999,
        "withdrawn_flag": 0,
        "black_box_warning": "0",
        "therapeutic_flag": true,
        "oral": 1,
        "molecule_structures": {
            "canonical_smiles": "CCO",
            "standard_inchi_key": "SYNTHETICINCHIKEY"
        },
        "molecule_properties": {
            "full_mwt": "46.07",
            "mw_freebase": "46.07",
            "alogp": "-0.31",
            "psa": "20.23",
            "hba": 1,
            "hbd": 1,
            "rtb": 0,
            "aromatic_rings": 0,
            "heavy_atoms": 3,
            "num_ro5_violations": 0,
            "qed_weighted": "0.48",
            "full_molformula": "C2H6O"
        },
        "molecule_synonyms": [{"molecule_synonym": "synthalin", "syn_type": "TRADE_NAME"}]
    })
}

#[derive(Clone, Default)]
struct Script {
    molecules: Vec<Value>,
    indications: Value,
    warnings: Value,
    activities: Value,
    mechanisms: Vec<Value>,
    targets: Value,
    status: Option<StatusCode>,
    retry_after: Option<&'static str>,
    raw: Option<String>,
}

impl Script {
    fn ok() -> Self {
        Self {
            molecules: vec![page("molecules", json!([invented_molecule()]), 1, None)],
            indications: empty_page("drug_indications"),
            warnings: empty_page("drug_warnings"),
            activities: empty_page("activities"),
            mechanisms: vec![empty_page("mechanisms")],
            targets: empty_page("targets"),
            status: None,
            retry_after: None,
            raw: None,
        }
    }
}

async fn serve(
    script: Script,
) -> (
    NativeBio,
    Arc<StdMutex<Vec<String>>>,
    tokio::task::JoinHandle<()>,
) {
    let captured = Arc::new(StdMutex::new(Vec::new()));
    let requests = captured.clone();
    let molecule_calls = Arc::new(StdMutex::new(0usize));
    let mechanism_calls = Arc::new(StdMutex::new(0usize));
    let app = Router::new().fallback(
        move |uri: Uri, Query(query): Query<HashMap<String, String>>| {
            let requests = requests.clone();
            let script = script.clone();
            let molecule_calls = molecule_calls.clone();
            let mechanism_calls = mechanism_calls.clone();
            async move {
                let mut recorded = uri.to_string();
                if !query.is_empty() && !recorded.contains('?') {
                    recorded.push('?');
                    recorded.push_str(
                        &query
                            .iter()
                            .map(|(key, value)| format!("{key}={value}"))
                            .collect::<Vec<_>>()
                            .join("&"),
                    );
                }
                requests.lock().unwrap().push(recorded);
                if let Some(status) = script.status {
                    let retry = script.retry_after.unwrap_or("60");
                    return (status, [("retry-after", retry)], "synthetic-key&value")
                        .into_response();
                }
                if let Some(raw) = script.raw {
                    return (StatusCode::OK, raw).into_response();
                }
                let path = uri.path();
                let body = if path.contains("/similarity/")
                    || path.contains("/substructure/")
                    || path.ends_with("/molecule.json")
                {
                    let mut n = molecule_calls.lock().unwrap();
                    let idx = (*n).min(script.molecules.len().saturating_sub(1));
                    *n += 1;
                    script
                        .molecules
                        .get(idx)
                        .cloned()
                        .unwrap_or_else(|| empty_page("molecules"))
                } else if path.ends_with("/drug_indication.json") {
                    script.indications.clone()
                } else if path.ends_with("/drug_warning.json") {
                    script.warnings.clone()
                } else if path.ends_with("/activity.json") {
                    script.activities.clone()
                } else if path.ends_with("/mechanism.json") {
                    let mut n = mechanism_calls.lock().unwrap();
                    let idx = (*n).min(script.mechanisms.len().saturating_sub(1));
                    *n += 1;
                    script
                        .mechanisms
                        .get(idx)
                        .cloned()
                        .unwrap_or_else(|| empty_page("mechanisms"))
                } else if path.ends_with("/target.json") {
                    script.targets.clone()
                } else {
                    json!({"error": "synthetic-key&value"})
                };
                (StatusCode::OK, axum::Json(body)).into_response()
            }
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (test_bio(&endpoint), captured, task)
}

#[test]
fn catalog_registers_six_chembl_tools() {
    let tools: Vec<_> = crate::catalog()
        .into_iter()
        .filter(|(domain, _)| *domain == "chembl")
        .collect();
    let names: Vec<_> = tools
        .iter()
        .map(|(_, schema)| schema.function.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "compound_search",
            "drug_search",
            "get_admet",
            "get_bioactivity",
            "get_mechanism",
            "target_search"
        ]
    );
    assert!(crate::selected_by_package("mcp_chembl"));
    assert!(crate::package_selects("mcp_bio", "chembl"));
    assert_eq!(
        crate::tools_for_package(Arc::new(NativeBio::new(&[]).unwrap()), "mcp_chembl").len(),
        6
    );
    assert_eq!(crate::domain_for_tool("compound_search"), Some("chembl"));
}

#[test]
fn rejects_out_of_bounds_and_unknown_fields() {
    assert!(serde_json::from_value::<CompoundSearch>(
        json!({"name": "x", "api_key": "synthetic-key&value"})
    )
    .is_err());
    assert!(serde_json::from_value::<GetAdmet>(
        json!({"molecule_chembl_id": "CHEMBL9990001", "api_key": "secret"})
    )
    .is_err());
    assert!(parse_chembl_id("CHEMBL0").is_err());
    assert!(parse_chembl_id("CHEMBL").is_err());
    assert_eq!(parse_chembl_id("chembl9990001").unwrap(), "CHEMBL9990001");
    assert!(bound_limit(0).is_err());
    assert!(bound_limit(101).is_err());
    assert!(bound_limit(1).is_ok());
    assert!(bound_phase(Some(5)).is_err());
    assert!(optional_smiles(Some("CC O")).is_err());
}

#[tokio::test]
async fn tool_arguments_are_rejected_before_http() {
    let bio = NativeBio::new(&[]).unwrap();
    for (name, args, expected) in [
        ("compound_search", json!({}), "requires name"),
        ("compound_search", json!({"name": "x", "limit": 0}), "limit"),
        (
            "compound_search",
            json!({"name": "x", "limit": 101}),
            "limit",
        ),
        (
            "compound_search",
            json!({"chembl_id": "CHEMBL0"}),
            "CHEMBL25",
        ),
        (
            "compound_search",
            json!({"smiles": "CCO", "similarity_threshold": 20}),
            "similarity_threshold",
        ),
        (
            "compound_search",
            json!({"similarity_threshold": 80}),
            "requires smiles",
        ),
        ("drug_search", json!({"indication": " "}), "indication"),
        (
            "get_admet",
            json!({"molecule_chembl_id": "not-an-id"}),
            "CHEMBL25",
        ),
        (
            "get_bioactivity",
            json!({"activity_type": "IC50"}),
            "molecule_chembl_id or target_chembl_id",
        ),
        (
            "get_bioactivity",
            json!({"molecule_chembl_id": "CHEMBL9990001", "min_pchembl": 20}),
            "min_pchembl",
        ),
        (
            "get_mechanism",
            json!({"action_type": "INHIBITOR"}),
            "molecule_chembl_id or target_chembl_id",
        ),
        ("target_search", json!({}), "requires"),
        (
            "target_search",
            json!({"target_type": "NOT A TYPE"}),
            "target_type",
        ),
    ] {
        let error = bio.call(name, &args).await.unwrap_err().to_string();
        assert!(error.contains(expected), "{name} {args} -> {error}");
        assert!(!error.contains("synthetic-key"));
    }
}

#[test]
fn page_parser_requires_collection_and_metadata() {
    let search = page("molecules", json!([invented_molecule()]), 3, Some("/next"));
    let parsed = parse_page(&search, "molecules").unwrap();
    assert_eq!(parsed.total, Some(3));
    assert!(parsed.has_more);
    assert!(parse_page(&json!({"molecules": []}), "molecules").is_err());
    assert!(parse_page(
        &json!({"page_meta": {"total_count": 1, "next": null}}),
        "molecules"
    )
    .is_err());
    assert!(parse_page(
        &json!({"molecules": [], "page_meta": {"total_count": "unknown", "next": null}}),
        "molecules"
    )
    .is_err());
    assert!(parse_page(
        &json!({"molecules": [], "page_meta": {"total_count": 4, "next": null}}),
        "molecules"
    )
    .is_err());
    assert!(parse_page(&json!({"error": "synthetic-key&value", "molecules": [], "page_meta": {"total_count": 0, "next": null}}), "molecules").is_err());
    let empty = parse_page(&empty_page("molecules"), "molecules").unwrap();
    assert_eq!(empty.returned_or_zero(), 0);
}

impl Page {
    fn returned_or_zero(&self) -> usize {
        self.records.len()
    }
}

#[tokio::test]
async fn compound_search_by_name_uses_shipped_dispatch_and_source_urls() {
    let (bio, captured, server) = serve(Script::ok()).await;
    let result = bio
        .call("compound_search", &json!({"name": "synthalin", "limit": 5}))
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["source"], "ChEMBL");
    assert_eq!(result["returned"], 1);
    assert_eq!(result["total"], 1);
    assert_eq!(result["has_more"], false);
    assert_eq!(
        result["compounds"][0]["molecule_chembl_id"],
        "CHEMBL9990001"
    );
    assert_eq!(
        result["compounds"][0]["url"],
        "https://www.ebi.ac.uk/chembl/compound_report_card/CHEMBL9990001/"
    );
    assert_eq!(result["compounds"][0]["properties"]["formula"], "C2H6O");
    assert_eq!(result["compounds"][0]["max_phase"], 4);
    let uris = captured.lock().unwrap().join(" ");
    assert!(uris.contains("/molecule.json"));
    assert!(uris.contains("molecule_synonyms__molecule_synonym__icontains=synthalin"));
    assert!(uris.contains("format=json"));
    assert!(uris.contains("limit=5"));
    assert!(!uris.contains("synthetic-key"));
    assert!(!result.to_string().contains("synthetic-key"));
    assert!(!result.to_string().contains("127.0.0.1"));
}

#[tokio::test]
async fn compound_search_falls_back_to_preferred_name() {
    let mut script = Script::ok();
    script.molecules = vec![
        empty_page("molecules"),
        page("molecules", json!([invented_molecule()]), 1, None),
    ];
    let (bio, captured, server) = serve(script).await;
    let result = bio
        .call("compound_search", &json!({"name": "synthalin"}))
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["returned"], 1);
    let uris = captured.lock().unwrap().join(" ");
    assert!(uris.contains("pref_name__icontains=synthalin"));
}

#[tokio::test]
async fn compound_search_encodes_smiles_on_similarity_path() {
    let mut molecule = invented_molecule();
    molecule["similarity"] = json!("82.5");
    let mut script = Script::ok();
    script.molecules = vec![page("molecules", json!([molecule]), 2, Some("/next"))];
    let (bio, captured, server) = serve(script).await;
    let result = bio
        .call(
            "compound_search",
            &json!({"smiles": "CC(=O)O", "similarity_threshold": 80, "limit": 1}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["query_kind"], "similarity");
    assert_eq!(result["has_more"], true);
    assert_eq!(result["compounds"][0]["similarity"], 82.5);
    let uris = captured.lock().unwrap().join(" ");
    assert!(uris.contains("/similarity/"));
    assert!(uris.contains("/80.json"));
    assert!(uris.contains("%28"));
    assert!(!uris.contains("order_by="));
}

#[tokio::test]
async fn drug_search_joins_indication_rows_by_parent_id() {
    let mut script = Script::ok();
    script.indications = page(
        "drug_indications",
        json!([{
            "drugind_id": 7,
            "molecule_chembl_id": "CHEMBL9990002",
            "parent_molecule_chembl_id": "CHEMBL9990001",
            "mesh_heading": "Invented fever",
            "efo_term": "synthetic hyperthermia",
            "max_phase_for_ind": "4.0"
        }]),
        8,
        Some("/next"),
    );
    script.warnings = page(
        "drug_warnings",
        json!([{
            "warning_id": 3,
            "parent_molecule_chembl_id": "CHEMBL9990001",
            "warning_type": "Black Box Warning",
            "warning_class": "Hepatic",
            "warning_country": "United States",
            "warning_year": "2012"
        }]),
        1,
        None,
    );
    let (bio, captured, server) = serve(script).await;
    let result = bio
        .call(
            "drug_search",
            &json!({"indication": "synthetic hyperthermia", "only_approved": true, "limit": 10}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["returned"], 1);
    assert_eq!(result["total"], 8);
    assert_eq!(result["has_more"], true);
    assert_eq!(result["match_field"], "efo_term");
    assert_eq!(result["drugs"][0]["molecule_chembl_id"], "CHEMBL9990001");
    assert_eq!(result["drugs"][0]["pref_name"], "SYNTHALIN");
    assert_eq!(result["drugs"][0]["best_phase_for_indication"], 4);
    assert_eq!(
        result["drugs"][0]["warnings"][0]["warning_class"],
        "Hepatic"
    );
    assert_eq!(
        result["drugs"][0]["url"],
        "https://www.ebi.ac.uk/chembl/compound_report_card/CHEMBL9990001/"
    );
    let uris = captured.lock().unwrap().join(" ");
    assert!(uris.contains("/drug_indication.json"));
    assert!(uris.contains("efo_term__icontains=synthetic"));
    assert!(uris.contains("max_phase_for_ind=4"));
    assert!(uris.contains("molecule_chembl_id__in=CHEMBL9990001"));
    assert!(uris.contains("/drug_warning.json"));
}

#[tokio::test]
async fn get_admet_reports_missing_molecules_without_inventing_properties() {
    let mut script = Script::ok();
    script.molecules = vec![empty_page("molecules")];
    let (bio, _, server) = serve(script).await;
    let result = bio
        .call("get_admet", &json!({"molecule_chembl_id": "chembl9990001"}))
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["found"], false);
    assert_eq!(result["properties"], Value::Null);
    assert_eq!(result["molecule_chembl_id"], "CHEMBL9990001");

    let (bio, captured, server) = serve(Script::ok()).await;
    let result = bio
        .call("get_admet", &json!({"molecule_chembl_id": "CHEMBL9990001"}))
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["found"], true);
    assert_eq!(result["properties"]["formula"], "C2H6O");
    assert_eq!(result["properties"]["full_mwt"], 46.07);
    assert!(captured
        .lock()
        .unwrap()
        .join(" ")
        .contains("molecule_chembl_id=CHEMBL9990001"));
}

#[tokio::test]
async fn get_bioactivity_and_target_search_preserve_identifiers() {
    let mut script = Script::ok();
    script.activities = page(
        "activities",
        json!([{
            "activity_id": "11",
            "molecule_chembl_id": "CHEMBL9990001",
            "target_chembl_id": "CHEMBL9991001",
            "target_pref_name": "Synthetic kinase",
            "target_organism": "Homo sapiens",
            "standard_type": "IC50",
            "standard_relation": "=",
            "standard_value": "12.0",
            "standard_units": "nM",
            "pchembl_value": "7.92",
            "assay_chembl_id": "CHEMBL9992001",
            "assay_type": "B",
            "assay_description": "Invented binding assay",
            "document_chembl_id": "CHEMBL9993001"
        }]),
        40,
        Some("/next"),
    );
    script.targets = page(
        "targets",
        json!([{
            "target_chembl_id": "CHEMBL9991001",
            "pref_name": "Synthetic kinase",
            "target_type": "SINGLE PROTEIN",
            "organism": "Homo sapiens",
            "tax_id": "9606",
            "target_components": [{
                "accession": "P00000",
                "target_component_synonyms": [
                    {"syn_type": "GENE_SYMBOL", "component_synonym": "SYN1"},
                    {"syn_type": "GENE_SYMBOL", "component_synonym": "SYN1"}
                ]
            }]
        }]),
        1,
        None,
    );
    let (bio, captured, server) = serve(script).await;
    let activities = bio
        .call(
            "get_bioactivity",
            &json!({
                "molecule_chembl_id": "CHEMBL9990001",
                "target_chembl_id": "CHEMBL9991001",
                "activity_type": "IC50",
                "min_pchembl": 6,
                "unit": "nM",
                "limit": 20
            }),
        )
        .await
        .unwrap();
    let targets = bio
        .call(
            "target_search",
            &json!({
                "gene_symbol": "SYN1",
                "organism": "Homo sapiens",
                "target_type": "SINGLE PROTEIN"
            }),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(activities["returned"], 1);
    assert_eq!(activities["has_more"], true);
    assert_eq!(activities["activities"][0]["pchembl_value"], 7.92);
    assert_eq!(
        activities["activities"][0]["target_url"],
        "https://www.ebi.ac.uk/chembl/target_report_card/CHEMBL9991001/"
    );
    assert_eq!(targets["targets"][0]["gene_symbols"], json!(["SYN1"]));
    assert_eq!(targets["targets"][0]["accessions"], json!(["P00000"]));
    let uris = captured.lock().unwrap().join(" ");
    assert!(uris.contains("pchembl_value__gte=6"));
    assert!(uris.contains("standard_type=IC50"));
    assert!(uris.contains("component_synonym__iexact=SYN1"));
    assert!(
        uris.contains("target_type=SINGLE+PROTEIN")
            || uris.contains("target_type=SINGLE%20PROTEIN")
    );
}

#[tokio::test]
async fn get_mechanism_retries_parent_identifier() {
    let mut script = Script::ok();
    script.mechanisms = vec![
        empty_page("mechanisms"),
        page(
            "mechanisms",
            json!([{
                "mec_id": 5,
                "molecule_chembl_id": "CHEMBL9990002",
                "parent_molecule_chembl_id": "CHEMBL9990001",
                "target_chembl_id": "CHEMBL9991001",
                "mechanism_of_action": "Synthetic kinase inhibitor",
                "action_type": "INHIBITOR",
                "direct_interaction": 1,
                "disease_efficacy": "true",
                "max_phase": 4
            }]),
            1,
            None,
        ),
    ];
    let (bio, captured, server) = serve(script).await;
    let result = bio
        .call(
            "get_mechanism",
            &json!({"molecule_chembl_id": "CHEMBL9990001", "action_type": "INHIBITOR"}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["returned"], 1);
    assert_eq!(result["mechanisms"][0]["direct_interaction"], true);
    assert_eq!(
        result["mechanisms"][0]["parent_molecule_chembl_id"],
        "CHEMBL9990001"
    );
    let uris = captured.lock().unwrap().join(" ");
    assert!(uris.contains("molecule_chembl_id=CHEMBL9990001"));
    assert!(uris.contains("parent_molecule_chembl_id=CHEMBL9990001"));
}

#[tokio::test]
async fn rejects_upstream_errors_and_malformed_json_without_echoing_secrets() {
    for (script, expected) in [
        (
            Script {
                status: Some(StatusCode::TOO_MANY_REQUESTS),
                retry_after: Some("60"),
                ..Script::ok()
            },
            "HTTP 429",
        ),
        (
            Script {
                raw: Some("synthetic-key&value {".into()),
                ..Script::ok()
            },
            "invalid JSON",
        ),
        (
            Script {
                raw: Some(json!({"error": "synthetic-key&value"}).to_string()),
                ..Script::ok()
            },
            "rejected",
        ),
        (
            Script {
                raw: Some(json!({"molecules": []}).to_string()),
                ..Script::ok()
            },
            "page metadata",
        ),
    ] {
        let (bio, _, server) = serve(script).await;
        let error = bio
            .call("compound_search", &json!({"name": "synthalin"}))
            .await
            .unwrap_err()
            .to_string();
        server.abort();
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("synthetic-key"));
        assert!(!error.contains("CHEMBL_API_KEY"));
    }
}

#[tokio::test]
async fn unknown_tool_stays_on_native_dispatch() {
    let bio = NativeBio::new(&[]).unwrap();
    let error = bio
        .call("compound_search_now", &json!({"name": "x"}))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown native biological tool"));
}

#[test]
fn percent_encodes_smiles_path_segments() {
    assert_eq!(path_segment("CC(=O)O"), "CC%28%3DO%29O");
    assert_eq!(path_segment("N#CC"), "N%23CC");
}
