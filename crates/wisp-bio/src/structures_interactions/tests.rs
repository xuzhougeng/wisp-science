use super::*;
use crate::http::{Http, MAX_RESPONSE};
use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

fn test_bio(base: &str) -> NativeBio {
    let base = base.trim_end_matches('/');
    NativeBio::test_client(
        &[
            (
                "PDB_SEARCH_URL".into(),
                format!("{base}/rcsbsearch/v2/query"),
            ),
            ("PDB_DATA_URL".into(), format!("{base}/rest/v1/core")),
            ("ALPHAFOLD_URL".into(), format!("{base}/api")),
            ("EMDB_URL".into(), format!("{base}/emdb/api")),
            (
                "COMPLEXPORTAL_URL".into(),
                format!("{base}/intact/complex-ws"),
            ),
            ("INTACT_URL".into(), format!("{base}/intact/ws")),
        ],
        Http(reqwest::Client::builder().no_proxy().build().unwrap()),
    )
    .unwrap()
}

async fn serve(app: Router) -> (NativeBio, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (test_bio(&endpoint), task)
}

fn pdb_entry(id: &str) -> Value {
    json!({
        "rcsb_id": id,
        "struct": {"title": "Synthetic p53 tetramer"},
        "exptl": [{"method": "X-RAY DIFFRACTION"}],
        "rcsb_entry_info": {
            "resolution_combined": [1.8],
            "structure_determination_methodology": "experimental",
            "molecular_weight": 86.2,
            "assembly_count": 1,
            "polymer_entity_count": 1,
            "polymer_entity_count_protein": 1,
            "nonpolymer_entity_count": 1,
            "nonpolymer_bound_components": ["ZN"]
        },
        "rcsb_accession_info": {
            "deposit_date": "1994-01-01",
            "initial_release_date": "1994-06-01",
            "status_code": "REL"
        },
        "rcsb_entry_container_identifiers": {
            "polymer_entity_ids": ["1"],
            "non_polymer_entity_ids": ["2"]
        },
        "rcsb_primary_citation": {
            "title": "Synthetic citation",
            "rcsb_journal_abbrev": "Nature",
            "year": 1994,
            "rcsb_authors": ["Doe J"],
            "pdbx_database_id_PubMed": 1,
            "pdbx_database_id_DOI": "10.example/synthetic"
        }
    })
}

fn success_app() -> Router {
    Router::new()
        .route(
            "/rcsbsearch/v2/query",
            get(|Query(params): Query<HashMap<String, String>>| async move {
                let payload: Value =
                    serde_json::from_str(params.get("json").map(String::as_str).unwrap_or("{}"))
                        .unwrap_or(json!({}));
                let start = payload
                    .pointer("/request_options/paginate/start")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                if start > 0 {
                    return axum::Json(json!({"total_count": 1, "result_set": []})).into_response();
                }
                axum::Json(json!({
                    "total_count": 1,
                    "result_set": [{"identifier": "1TUP", "score": 1.0}]
                }))
                .into_response()
            }),
        )
        .route(
            "/rest/v1/core/entry/{id}",
            get(|Path(id): Path<String>| async move {
                if id == "ZZZZ" {
                    StatusCode::NOT_FOUND.into_response()
                } else {
                    axum::Json(pdb_entry(&id)).into_response()
                }
            }),
        )
        .route(
            "/rest/v1/core/polymer_entity/{pdb}/{entity}",
            get(|Path((pdb, entity)): Path<(String, String)>| async move {
                if entity == "9" {
                    return StatusCode::NOT_FOUND.into_response();
                }
                axum::Json(json!({
                    "rcsb_id": format!("{pdb}_{entity}"),
                    "rcsb_polymer_entity": {"pdbx_description": "Cellular tumor antigen p53"},
                    "rcsb_polymer_entity_container_identifiers": {
                        "entry_id": pdb,
                        "entity_id": entity,
                        "asym_ids": ["A"],
                        "auth_asym_ids": ["A"],
                        "uniprot_ids": ["P04637"],
                        "reference_sequence_identifiers": [{
                            "database_name": "UniProt",
                            "database_accession": "P04637",
                            "entity_sequence_coverage": 100.0
                        }]
                    },
                    "entity_poly": {
                        "rcsb_entity_polymer_type": "Protein",
                        "rcsb_sample_sequence_length": 6,
                        "pdbx_seq_one_letter_code_can": "MEEPQS"
                    },
                    "rcsb_entity_source_organism": [{"scientific_name": "Homo sapiens", "ncbi_taxonomy_id": 9606}]
                }))
                .into_response()
            }),
        )
        .route(
            "/rest/v1/core/nonpolymer_entity/{pdb}/{entity}",
            get(|Path((_pdb, entity)): Path<(String, String)>| async move {
                axum::Json(json!({
                    "rcsb_nonpolymer_entity_container_identifiers": {
                        "entity_id": entity,
                        "nonpolymer_comp_id": "ZN",
                        "auth_asym_ids": ["A"]
                    },
                    "rcsb_nonpolymer_entity": {
                        "pdbx_description": "ZINC ION",
                        "pdbx_number_of_molecules": 1
                    }
                }))
                .into_response()
            }),
        )
        .route(
            "/rest/v1/core/chemcomp/{id}",
            get(|Path(id): Path<String>| async move {
                axum::Json(json!({
                    "chem_comp": {"id": id, "name": "ZINC ION", "formula": "Zn", "formula_weight": 65.38, "type": "ION"},
                    "rcsb_chem_comp_descriptor": {"InChIKey": "PTFCDOFLOPIGGS-UHFFFAOYSA-N", "SMILES": "[Zn+2]"}
                }))
                .into_response()
            }),
        )
        .route(
            "/api/prediction/{accession}",
            get(|Path(accession): Path<String>| async move {
                match accession.as_str() {
                    "MISSING1" => StatusCode::NOT_FOUND.into_response(),
                    "BADID" => StatusCode::BAD_REQUEST.into_response(),
                    _ => axum::Json(json!([{
                        "modelEntityId": format!("AF-{accession}-F1"),
                        "entryId": format!("AF-{accession}-F1"),
                        "uniprotAccession": accession,
                        "uniprotId": "P53_HUMAN",
                        "uniprotDescription": "Cellular tumor antigen p53",
                        "gene": "TP53",
                        "organismScientificName": "Homo sapiens",
                        "taxId": 9606,
                        "globalMetricValue": 75.1,
                        "fractionPlddtVeryHigh": 0.5,
                        "latestVersion": 6,
                        "sequence": "MEEPQSD",
                        "cifUrl": "https://alphafold.ebi.ac.uk/files/synthetic.cif"
                    }]))
                    .into_response(),
                }
            }),
        )
        .route(
            "/emdb/api/entry/{id}",
            get(|Path(id): Path<String>| async move {
                if id == "EMD-0" {
                    return StatusCode::NOT_FOUND.into_response();
                }
                axum::Json(json!({
                    "emdb_id": id,
                    "admin": {
                        "title": "Synthetic apoferritin",
                        "current_status": {"code": "REL"},
                        "key_dates": {"deposition": "2020-01-01T00:00:00", "map_release": "2020-02-01"}
                    },
                    "structure_determination_list": {
                        "structure_determination": [{
                            "method": "singleParticle",
                            "image_processing": [{"final_reconstruction": {"resolution": {"valueOf_": 1.25, "units": "A"}, "resolution_method": "FSC 0.143"}}]
                        }]
                    },
                    "sample": {"name": {"valueOf_": "apoferritin"}},
                    "crossreferences": {
                        "pdb_list": {"pdb_reference": [{"pdb_id": "7a4m"}]},
                        "citation_list": {"primary_citation": {"citation_type": {
                            "title": "Synthetic EM paper",
                            "journal_abbreviation": "Nature",
                            "year": 2020,
                            "author": [{"valueOf_": "Doe J", "order": 1}],
                            "external_references": [{"type_": "DOI", "valueOf_": "10.example/em"}]
                        }}}
                    },
                    "map": {"file": "emd_8888.map.gz", "dimensions": {"col": 10, "row": 10, "sec": 10}, "pixel_spacing": {"x": {"valueOf_": 1.0, "units": "A"}}}
                }))
                .into_response()
            }),
        )
        .route(
            "/emdb/api/analysis/{id}",
            get(|Path(id): Path<String>| async move {
                let num = id.rsplit('-').next().unwrap_or(&id).to_string();
                axum::Json(json!({
                    num.clone(): {
                        "qscore": {"allmodels_average_qscore": 0.6},
                        "atom_inclusion_by_level": {"average_ai_allmodels": 0.8},
                        "resolution": {"value": 1.3}
                    }
                }))
                .into_response()
            }),
        )
        .route(
            "/emdb/api/search/{query}",
            get(|Path(_query): Path<String>| async move {
                axum::Json(json!([
                    {"emdb_id": "EMD-8888", "title": "Synthetic apoferritin", "resolution": 1.25, "structure_determination_method": "singleParticle", "current_status": "REL", "fitted_pdbs": "7a4m"},
                    {"emdb_id": "EMD-1", "title": "Obsolete", "current_status": "OBS"}
                ]))
                .into_response()
            }),
        )
        .route(
            "/emdb/api/facet/{query}",
            get(|Path(_query): Path<String>| async move {
                axum::Json(json!({"current_status": {"REL": 1}})).into_response()
            }),
        )
        .route(
            "/intact/complex-ws/complex/{ac}",
            get(|Path(ac): Path<String>| async move {
                if ac == "CPX-0" {
                    return StatusCode::NOT_FOUND.into_response();
                }
                axum::Json(json!({
                    "complexAc": ac,
                    "ac": "EBI-900000",
                    "name": "p53 tetramer",
                    "systematicName": "TP53 tetramer",
                    "species": "Homo sapiens; 9606",
                    "evidenceType": {"identifier": "ECO:0000353", "description": "physical interaction evidence"},
                    "participants": [{
                        "identifier": "P04637",
                        "name": "P53_HUMAN",
                        "interactorType": "protein",
                        "bioRole": "unspecified role",
                        "stochiometry": "minValue: 4, maxValue: 4"
                    }],
                    "crossReferences": [{
                        "database": "gene ontology",
                        "identifier": "GO:0000733",
                        "qualifier": "biological process",
                        "description": "DNA damage response"
                    }]
                }))
                .into_response()
            }),
        )
        .route(
            "/intact/complex-ws/search/{query}",
            get(|Path(_query): Path<String>, Query(params): Query<HashMap<String, String>>| async move {
                let _ = params;
                axum::Json(json!({
                    "totalNumberOfResults": 1,
                    "elements": [{"complexAC": "CPX-2158", "complexName": "p53 tetramer", "organismName": "Homo sapiens; 9606"}]
                }))
                .into_response()
            }),
        )
        .route(
            "/intact/ws/interaction/findInteractionWithFacet",
            post(|body: String| async move {
                let partner = if body.contains("query=Q9") { "P04637" } else { "Q9AAA1" };
                axum::Json(json!({
                    "data": {
                        "totalElements": 1,
                        "last": true,
                        "content": [{
                            "ac": "EBI-1",
                            "binaryInteractionId": 11,
                            "idA": "P04637 (uniprotkb)",
                            "idB": format!("{partner} (uniprotkb)"),
                            "moleculeA": "P53",
                            "moleculeB": "MDM2",
                            "type": "physical association",
                            "detectionMethod": "anti tag coimmunoprecipitation",
                            "intactMiscore": 0.72,
                            "publicationPubmedIdentifier": "1",
                            "speciesA": "Homo sapiens",
                            "taxIdA": 9606
                        }]
                    }
                }))
                .into_response()
            }),
        )
        .route(
            "/intact/ws/interactor/findInteractor/{query}",
            get(|Path(query): Path<String>| async move {
                axum::Json(json!({
                    "last": true,
                    "content": [{
                        "interactorAc": "EBI-7090529",
                        "interactorPreferredIdentifier": format!("{query} (uniprotkb)"),
                        "interactorName": "P53_HUMAN",
                        "interactorSpecies": "Homo sapiens",
                        "interactorTaxId": 9606,
                        "interactorType": "protein",
                        "interactionCount": 12
                    }]
                }))
                .into_response()
            }),
        )
        .route(
            "/intact/ws/graph/interaction/details/{ac}",
            get(|Path(ac): Path<String>| async move {
                if ac == "EBI-0" {
                    return (StatusCode::OK, "").into_response();
                }
                axum::Json(json!({
                    "interactionAc": ac,
                    "shortLabel": "p53-mdm2",
                    "type": {"shortName": "physical association", "identifier": "MI:0915"},
                    "detectionMethod": {"shortName": "anti tag coip", "identifier": "MI:0007"},
                    "hostOrganism": "Homo sapiens",
                    "publication": {"pubmedId": "1", "title": "Synthetic", "year": 2020}
                }))
                .into_response()
            }),
        )
        .route(
            "/intact/ws/graph/participants/details/{ac}",
            get(|Path(_ac): Path<String>| async move {
                axum::Json(json!({
                    "last": true,
                    "content": [{
                        "participantAc": "EBI-p1",
                        "shortLabel": "p53",
                        "participantId": {"identifier": "P04637", "database": {"shortName": "uniprotkb"}},
                        "type": {"shortName": "protein", "identifier": "MI:0326"},
                        "species": {"scientificName": "Homo sapiens", "taxId": 9606}
                    }]
                }))
                .into_response()
            }),
        )
}

#[test]
fn catalog_registers_sixteen_structures_interactions_tools() {
    let names: Vec<_> = catalog()
        .into_iter()
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        names,
        vec![
            ("structures-interactions", "alphafold_check_coverage".into()),
            ("structures-interactions", "alphafold_get_prediction".into()),
            (
                "structures-interactions",
                "complexportal_get_complexes".into()
            ),
            (
                "structures-interactions",
                "complexportal_search_by_participant".into()
            ),
            ("structures-interactions", "emdb_get_entries".into()),
            ("structures-interactions", "emdb_get_entry_section".into()),
            ("structures-interactions", "emdb_get_validation".into()),
            ("structures-interactions", "emdb_search_entries".into()),
            ("structures-interactions", "intact_build_network".into()),
            (
                "structures-interactions",
                "intact_fetch_interactions".into()
            ),
            (
                "structures-interactions",
                "intact_get_interaction_details".into()
            ),
            ("structures-interactions", "intact_get_interactor".into()),
            ("structures-interactions", "pdb_get_entities".into()),
            ("structures-interactions", "pdb_get_ligands".into()),
            ("structures-interactions", "pdb_get_structures".into()),
            ("structures-interactions", "pdb_search_structures".into()),
        ]
    );
    assert!(crate::contains_tool("pdb_search_structures"));
    assert_eq!(
        crate::domain_for_tool("alphafold_get_prediction"),
        Some("structures-interactions")
    );
    assert!(crate::package_selects(
        "mcp_structures_interactions",
        "structures-interactions"
    ));
    assert!(crate::selected_by_package("mcp_structures_interactions"));
}

#[test]
fn rejects_unbounded_or_malformed_arguments() {
    assert!(pdb::fold_pdb_id("1tup").unwrap() == Some("1TUP".into()));
    assert!(pdb::fold_pdb_id("../etc").is_err());
    assert!(alphafold::fold_uniprot("p04637").unwrap() == Some("P04637".into()));
    assert!(alphafold::fold_uniprot("bad id").is_err());
    assert!(emdb::fold_emdb_id("1234").unwrap() == Some("EMD-1234".into()));
    assert!(emdb::fold_emdb_id("emd-88").unwrap() == Some("EMD-88".into()));
    assert!(emdb::fold_emdb_id("not-an-id").is_err());
    assert!(complexportal::fold_complex_ac("cpx-2158").unwrap() == Some("CPX-2158".into()));
    assert!(complexportal::fold_complex_ac("P04637").is_err());
    let search: pdb::Search = serde_json::from_value(json!({})).unwrap();
    assert!(pdb::build_search_query(&search).is_err());
    assert!(
        serde_json::from_value::<pdb::Search>(json!({"text": "p53", "api_key": "secret"})).is_err()
    );
    let too_many: Vec<String> = (0..26).map(|i| format!("1TU{i:X}")).collect();
    assert!(unique_ids(&too_many, 25, "PDB identifier", pdb::fold_pdb_id).is_err());
    assert!(bound_int(0, 1, 100, "max_rows").is_err());
    assert!(bound_score(1.2, "min_mi_score").is_err());
}

#[tokio::test]
async fn each_source_dispatches_through_native_bio_call_with_source_urls() {
    let (bio, server) = serve(success_app()).await;
    let pdb = bio
        .call(
            "pdb_get_structures",
            &json!({"pdb_ids": ["1tup", "1TUP", ""]}),
        )
        .await
        .unwrap();
    let search = bio
        .call(
            "pdb_search_structures",
            &json!({"uniprot_accession": "P04637", "max_rows": 10}),
        )
        .await
        .unwrap();
    let af = bio
        .call(
            "alphafold_get_prediction",
            &json!({"uniprot_accession": "P04637", "include_sequence": true}),
        )
        .await
        .unwrap();
    let emdb = bio
        .call("emdb_get_entries", &json!({"emdb_ids": ["8888", "EMD-0"]}))
        .await
        .unwrap();
    let cpx = bio
        .call(
            "complexportal_get_complexes",
            &json!({"complex_acs": ["CPX-2158", "CPX-0"]}),
        )
        .await
        .unwrap();
    let intact = bio
        .call(
            "intact_fetch_interactions",
            &json!({"query": "P04637", "min_mi_score": 0.45, "max_records_returned": 10}),
        )
        .await
        .unwrap();
    server.abort();

    assert_eq!(pdb["source"], "RCSB PDB");
    assert_eq!(pdb["source_url"], PDB_SITE);
    assert_eq!(pdb["n_unique"], 1);
    assert_eq!(pdb["n_blank_skipped"], 1);
    assert_eq!(pdb["n_duplicate_skipped"], 1);
    assert_eq!(
        pdb["records"][0]["url"],
        "https://www.rcsb.org/structure/1TUP"
    );
    assert_eq!(pdb["records"][0]["resolution_angstrom"], 1.8);

    assert_eq!(search["total_count"], 1);
    assert_eq!(search["records"][0]["pdb_id"], "1TUP");
    assert_eq!(search["truncated"], false);

    assert_eq!(af["source"], "AlphaFold DB");
    assert_eq!(af["source_url"], ALPHAFOLD_SITE);
    assert_eq!(af["has_model"], true);
    assert_eq!(af["url"], "https://alphafold.ebi.ac.uk/entry/P04637");
    assert_eq!(
        af["models"][0]["urls"]["cif"],
        "https://alphafold.ebi.ac.uk/files/synthetic.cif"
    );
    assert_eq!(af["models"][0]["sequence"], "MEEPQSD");

    assert_eq!(emdb["source"], "EMDB");
    assert_eq!(emdb["source_url"], EMDB_SITE);
    assert_eq!(
        emdb["records"][0]["url"],
        "https://www.ebi.ac.uk/emdb/EMD-8888"
    );
    assert_eq!(emdb["records"][0]["resolution_angstrom"], 1.25);
    assert_eq!(emdb["records"][1]["error"], "not_found");

    assert_eq!(cpx["source"], "Complex Portal");
    assert_eq!(
        cpx["records"][0]["url"],
        "https://www.ebi.ac.uk/complexportal/complex/CPX-2158"
    );
    assert_eq!(cpx["records"][0]["participants"][0]["stoichiometry_min"], 4);
    assert_eq!(cpx["not_found"], json!(["CPX-0"]));

    assert_eq!(intact["source"], "IntAct");
    assert_eq!(intact["source_url"], INTACT_SITE);
    assert_eq!(intact["records"][0]["id_a"], "P04637");
    assert_eq!(
        intact["records"][0]["url"],
        "https://www.ebi.ac.uk/intact/details/interaction/EBI-1"
    );
    assert_eq!(intact["truncated"], false);
}

#[tokio::test]
async fn remaining_tools_cover_missing_truncation_and_sections() {
    let (bio, server) = serve(success_app()).await;
    let coverage = bio
        .call(
            "alphafold_check_coverage",
            &json!({"uniprot_accessions": ["P04637", "MISSING1", "BADID"]}),
        )
        .await
        .unwrap();
    let entities = bio
        .call(
            "pdb_get_entities",
            &json!({"pdb_id": "1TUP", "entity_ids": ["1", "9"], "include_sequences": true}),
        )
        .await
        .unwrap();
    let ligands = bio
        .call("pdb_get_ligands", &json!({"pdb_id": "1TUP"}))
        .await
        .unwrap();
    let section = bio
        .call(
            "emdb_get_entry_section",
            &json!({"emdb_ids": ["EMD-8888"], "section": "publications"}),
        )
        .await
        .unwrap();
    let validation = bio
        .call("emdb_get_validation", &json!({"emdb_ids": ["EMD-8888"]}))
        .await
        .unwrap();
    let emdb_search = bio
        .call(
            "emdb_search_entries",
            &json!({"query": "title:apoferritin", "max_rows": 1}),
        )
        .await
        .unwrap();
    let cpx_search = bio
        .call(
            "complexportal_search_by_participant",
            &json!({"accession": "P04637"}),
        )
        .await
        .unwrap();
    let interactor = bio
        .call("intact_get_interactor", &json!({"query": "P04637"}))
        .await
        .unwrap();
    let details = bio
        .call(
            "intact_get_interaction_details",
            &json!({"interaction_ac": "EBI-1"}),
        )
        .await
        .unwrap();
    let missing_details = bio
        .call(
            "intact_get_interaction_details",
            &json!({"interaction_ac": "EBI-0"}),
        )
        .await
        .unwrap();
    let network = bio
        .call(
            "intact_build_network",
            &json!({"seed_accessions": ["P04637"], "max_interactors_expanded": 1}),
        )
        .await
        .unwrap();
    let missing_pdb = bio
        .call("pdb_get_structures", &json!({"pdb_ids": ["zzzz"]}))
        .await
        .unwrap();
    server.abort();

    assert_eq!(coverage["records"][1]["has_model"], false);
    assert!(coverage["records"][2]["error"]
        .as_str()
        .unwrap()
        .contains("invalid_accession"));
    assert_eq!(entities["not_found"], json!(["9"]));
    assert_eq!(entities["records"][0]["sequence"], "MEEPQS");
    assert_eq!(ligands["ligands"][0]["chem_comp"]["comp_id"], "ZN");
    assert_eq!(
        section["records"][0]["primary_citation"]["doi"],
        "10.example/em"
    );
    assert_eq!(validation["records"][0]["has_validation_analysis"], true);
    assert_eq!(emdb_search["returned"], 1);
    assert_eq!(emdb_search["truncated"], true);
    assert_eq!(cpx_search["complexes"][0]["complex_ac"], "CPX-2158");
    assert_eq!(interactor["n_matches"], 1);
    assert_eq!(details["participants"][0]["identifier"], "P04637");
    assert_eq!(missing_details["error"], "not_found");
    assert_eq!(network["n_nodes"], 2);
    assert_eq!(network["expansion"]["complete"], true);
    assert_eq!(missing_pdb["records"][0]["error"], "not_found");
}

#[tokio::test]
async fn rejects_rate_limits_malformed_json_and_oversize_without_echoing_secrets() {
    for (status, body, expected) in [
        (
            StatusCode::TOO_MANY_REQUESTS,
            "secret-token".into(),
            "HTTP 429",
        ),
        (StatusCode::OK, "{not-json".into(), "invalid JSON"),
        (
            StatusCode::OK,
            " ".repeat(MAX_RESPONSE + 1),
            "exceeded 4 MiB",
        ),
    ] {
        let app = Router::new().route(
            "/api/prediction/{accession}",
            get({
                let body = body.clone();
                move |Path(_accession): Path<String>| {
                    let body = body.clone();
                    async move { (status, [("retry-after", "60")], body).into_response() }
                }
            }),
        );
        let (bio, server) = serve(app).await;
        let error = bio
            .call(
                "alphafold_get_prediction",
                &json!({"uniprot_accession": "P04637"}),
            )
            .await
            .unwrap_err()
            .to_string();
        server.abort();
        assert!(
            error.contains(expected),
            "{error} did not contain {expected}"
        );
        assert!(!error.contains("secret-token"), "{error}");
    }
}

#[tokio::test]
async fn pdb_search_empty_204_is_not_an_error() {
    let app = Router::new().route(
        "/rcsbsearch/v2/query",
        get(|| async { StatusCode::NO_CONTENT }),
    );
    let (bio, server) = serve(app).await;
    let result = bio
        .call("pdb_search_structures", &json!({"text": "no-such-fold"}))
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["total_count"], 0);
    assert_eq!(result["returned"], 0);
    assert_eq!(result["truncated"], false);
}

#[tokio::test]
async fn unknown_tool_name_is_rejected() {
    let (bio, server) = serve(Router::new()).await;
    let error = call(&bio, "pdb_not_a_tool", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("unknown native biological tool"));
}

#[tokio::test]
async fn captured_pdb_search_encodes_documented_filters() {
    let captured = Arc::new(StdMutex::new(String::new()));
    let body = captured.clone();
    let app = Router::new().route(
        "/rcsbsearch/v2/query",
        get(move |Query(params): Query<HashMap<String, String>>| {
            let body = body.clone();
            async move {
                *body.lock().unwrap() = params.get("json").cloned().unwrap_or_default();
                axum::Json(json!({"total_count": 0, "result_set": []}))
            }
        }),
    );
    let (bio, server) = serve(app).await;
    bio.call(
        "pdb_search_structures",
        &json!({
            "text": "p53 DNA",
            "organism": "Homo sapiens",
            "experimental_method": "x-ray diffraction",
            "max_resolution_angstrom": 2.5,
            "include_computed_models": true
        }),
    )
    .await
    .unwrap();
    server.abort();
    let payload = captured.lock().unwrap().clone();
    assert!(payload.contains("full_text"), "{payload}");
    assert!(payload.contains("taxonomy_lineage.name"), "{payload}");
    assert!(payload.contains("X-RAY DIFFRACTION"), "{payload}");
    assert!(payload.contains("computational"), "{payload}");
}
