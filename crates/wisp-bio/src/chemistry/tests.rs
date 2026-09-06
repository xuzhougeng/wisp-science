use crate::http::Http;
use crate::NativeBio;
use axum::{
    body::Bytes,
    extract::State,
    http::{Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    Router,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct Fake {
    captured: Arc<Mutex<String>>,
    forced: Option<(u16, String)>,
}

#[test]
fn catalog_registers_twelve_chemistry_tools() {
    let names: Vec<_> = crate::catalog()
        .into_iter()
        .filter(|(domain, _)| *domain == "chemistry")
        .map(|(_, schema)| schema.function.name)
        .collect();
    assert_eq!(
        names,
        [
            "pubchem_search_compounds",
            "pubchem_get_compounds",
            "pubchem_similarity_search",
            "pubchem_get_bioassay_summary",
            "pubchem_get_safety",
            "chebi_search",
            "chebi_get_entity",
            "chebi_get_ontology",
            "rhea_search_reactions",
            "rhea_get_reaction",
            "bindingdb_ligands_by_target",
            "bindingdb_targets_by_compound",
        ]
    );
    let tools = crate::tools_for_package(
        Arc::new(crate::NativeBio::new(&[]).unwrap()),
        "mcp_chemistry",
    );
    assert_eq!(tools.len(), 12);
    assert!(crate::package_selects("mcp_chemistry", "chemistry"));
    assert!(crate::contains_tool("pubchem_search_compounds"));
    assert_eq!(
        crate::domain_for_tool("bindingdb_targets_by_compound"),
        Some("chemistry")
    );
}

#[tokio::test]
async fn argument_bounds_and_unknown_fields() {
    let bio = empty_bio();
    for args in [
        json!({"query": ""}),
        json!({"query": "x", "max_cids": 0}),
        json!({"query": "x", "max_cids": 101}),
        json!({"query": "x", "namespace": "inchi"}),
        json!({"query": "x", "api_key": "secret"}),
    ] {
        assert!(
            super::pubchem::search_compounds(&bio, &args).await.is_err(),
            "{args}"
        );
    }
    assert!(super::pubchem::get_compounds(&bio, &json!({"cids": []}))
        .await
        .is_err());
    assert!(super::pubchem::get_compounds(&bio, &json!({"cids": [0]}))
        .await
        .is_err());
    assert_eq!(
        super::chebi::normalize_chebi_id("CHEBI:424242").unwrap(),
        424242
    );
    assert!(super::chebi::normalize_chebi_id("not-an-id").is_err());
    assert_eq!(
        super::rhea::normalize_rhea_id("10280").unwrap(),
        "RHEA:10280"
    );
    assert!(
        super::rhea::search_reactions(&bio, &json!({"query": "2.1.1.-"}))
            .await
            .unwrap_err()
            .to_string()
            .contains("complete EC number")
    );
    assert!(
        super::bindingdb::ligands_by_target(&bio, &json!({"uniprot": "not-uniprot"}))
            .await
            .is_err()
    );
    assert!(super::bindingdb::targets_by_compound(
        &bio,
        &json!({"smiles": "CCO", "similarity": 0.1})
    )
    .await
    .is_err());
}

fn empty_bio() -> NativeBio {
    NativeBio::new(&[]).unwrap()
}

async fn data_client() -> (NativeBio, Arc<Mutex<String>>, tokio::task::JoinHandle<()>) {
    spawn(None).await
}

async fn status_client(
    status: u16,
    body: &str,
) -> (NativeBio, Arc<Mutex<String>>, tokio::task::JoinHandle<()>) {
    spawn(Some((status, body.to_string()))).await
}

async fn spawn(
    forced: Option<(u16, String)>,
) -> (NativeBio, Arc<Mutex<String>>, tokio::task::JoinHandle<()>) {
    let captured = Arc::new(Mutex::new(String::new()));
    let fake = Fake {
        captured: captured.clone(),
        forced,
    };
    let app = Router::new().fallback(dispatch).with_state(fake);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let http = Http(reqwest::Client::builder().no_proxy().build().unwrap());
    let bio = NativeBio::test_client(
        &[
            ("NCBI_EMAIL".into(), "operator@example.test".into()),
            ("WISP_BIO_TEST_PUBCHEM".into(), format!("{origin}/rest/pug")),
            (
                "WISP_BIO_TEST_PUBCHEM_VIEW".into(),
                format!("{origin}/rest/pug_view"),
            ),
            (
                "WISP_BIO_TEST_CHEBI".into(),
                format!("{origin}/chebi/backend/api/public"),
            ),
            ("WISP_BIO_TEST_RHEA".into(), format!("{origin}/sparql")),
            ("WISP_BIO_TEST_BINDINGDB".into(), format!("{origin}/rest")),
        ],
        http,
    )
    .unwrap();
    (bio, captured, server)
}

async fn dispatch(State(fake): State<Fake>, method: Method, uri: Uri, body: Bytes) -> Response {
    let text = String::from_utf8_lossy(&body).into_owned();
    *fake.captured.lock().unwrap() = format!("{method} {uri} {text}");
    if let Some((code, body)) = &fake.forced {
        return (
            StatusCode::from_u16(*code).unwrap(),
            [("retry-after", "60")],
            body.clone(),
        )
            .into_response();
    }
    route(uri.path(), uri.query().unwrap_or(""), &text)
}

fn route(path: &str, query: &str, body: &str) -> Response {
    let form = parse_form(body);
    let q = parse_form(query);
    if path.ends_with("/sparql") || path == "/sparql" {
        return rhea_sparql(body);
    }
    if path.contains("/compound/")
        && path.ends_with("/cids/JSON")
        && path.contains("fastsimilarity")
    {
        return json_ok(json!({"IdentifierList": {"CID": [424242, 424243]}}));
    }
    if path.contains("/compound/") && path.ends_with("/cids/JSON") {
        let query_value = form
            .get("name")
            .or_else(|| form.get("smiles"))
            .or_else(|| form.get("inchikey"))
            .or_else(|| form.get("cid"))
            .cloned()
            .unwrap_or_default();
        if query_value.contains("missing") {
            return not_found();
        }
        return json_ok(json!({"IdentifierList": {"CID": [424242, 424243]}}));
    }
    if path.contains("/property/") {
        let cids = form.get("cid").cloned().unwrap_or_default();
        if !cids.split(',').any(|cid| cid == "424242") {
            return not_found();
        }
        return json_ok(json!({
            "PropertyTable": {"Properties": [{
                "CID": 424242,
                "MolecularFormula": "C9H8O4",
                "SMILES": "CC(=O)OC1=CC=CC=C1C(=O)O",
                "IUPACName": "synthetic-amide"
            }, {"CID": 424243}]}
        }));
    }
    if path.contains("/synonyms/") {
        return json_ok(json!({
            "InformationList": {"Information": [{
                "CID": 424242,
                "Synonym": ["synthetic-amide", "invented-brand", "extra-synonym"]
            }]}
        }));
    }
    if path.contains("/assaysummary/") {
        if path.contains("/999999/") {
            return not_found();
        }
        return json_ok(json!({
            "Table": {
                "Columns": {"Column": ["AID", "CID", "Activity Outcome", "Assay Name"]},
                "Row": [
                    {"Cell": [1, 424242, "Active", "invented assay"]},
                    {"Cell": [2, 424242, "Inactive", "second assay"]}
                ]
            }
        }));
    }
    if path.contains("/pug_view/") {
        if path.contains("/999999/") {
            return not_found();
        }
        return json_ok(json!({
            "Record": {
                "RecordNumber": 424242,
                "RecordTitle": "synthetic-amide",
                "Section": [{
                    "TOCHeading": "Safety and Hazards",
                    "Section": [{
                        "TOCHeading": "GHS Classification",
                        "Information": [
                            {"Name": "Signal", "Value": {"StringWithMarkup": [{"String": "Danger"}]}},
                            {"Name": "Pictogram(s)", "Value": {"StringWithMarkup": [{
                                "String": "flame",
                                "Markup": [{"Extra": "Irritant"}]
                            }]}},
                            {"Name": "GHS Hazard Statements", "Value": {"StringWithMarkup": [{
                                "String": "H302 (100%): Harmful if swallowed"
                            }]}}
                        ]
                    }]
                }],
                "Reference": [{"SourceName": "invented-sds"}]
            }
        }));
    }
    if path.contains("/es_search") {
        let term = q.get("term").cloned().unwrap_or_default();
        if term.contains("missing") {
            return not_found();
        }
        return json_ok(json!({
            "total": 3,
            "number_pages": 1,
            "results": [{
                "_score": 12.5,
                "_source": {
                    "chebi_accession": "CHEBI:424242",
                    "name": "synthetic-amide",
                    "formula": "C9H8O4",
                    "inchikey": "AAAAAAAAAAAAAA-UHFFFAOYSA-N"
                }
            }]
        }));
    }
    if path.contains("/compound/") {
        let id = path.rsplit('/').find(|part| !part.is_empty()).unwrap_or("");
        if id != "424242" {
            return not_found();
        }
        return json_ok(chebi_compound());
    }
    if path.ends_with("/getLigandsByUniprots") {
        let uniprot = q.get("uniprot").cloned().unwrap_or_default();
        if uniprot.contains("P00000") {
            return json_ok(json!({"getLindsByUniprotsResponse": {"affinities": []}}));
        }
        return json_ok(json!({
            "getLindsByUniprotsResponse": {
                "affinities": [
                    {
                        "query": "synthetic-kinase",
                        "monomerid": 77,
                        "smile": "CCO",
                        "affinity_type": "Ki",
                        "affinity": "50",
                        "pmid": "11111111",
                        "doi": "10.example/synthetic"
                    },
                    {
                        "query": "synthetic-kinase",
                        "monomerid": 88,
                        "smile": "CCN",
                        "affinity_type": "Ki",
                        "affinity": "5",
                        "pmid": "",
                        "doi": ""
                    }
                ]
            }
        }));
    }
    if path.ends_with("/getTargetByCompound") {
        return json_ok(json!({
            "getTargetByCompoundResponse": {
                "bdb.hit": 1,
                "bdb.affinities": [{
                    "bdb.monomerid": 77,
                    "bdb.smiles": "CCO",
                    "bdb.inhibitor": "synthetic-ligand",
                    "bdb.target": "synthetic-kinase",
                    "bdb.species": "Homo sapiens",
                    "bdb.affinity_type": "Kd",
                    "bdb.affinity": "8",
                    "bdb.tanimoto": "0.91"
                }]
            }
        }));
    }
    (StatusCode::NOT_FOUND, "no fake route").into_response()
}

fn chebi_compound() -> Value {
    json!({
        "chebi_accession": "CHEBI:424242",
        "name": "synthetic-amide",
        "definition": "An invented amide used in tests.",
        "stars": 3,
        "names": {
            "SYNONYM": [{"name": "invented-brand"}, {"name": "extra-synonym"}],
            "IUPAC NAME": [{"name": "synthetic-amide"}]
        },
        "chemical_data": {"formula": "C9H8O4", "charge": 0, "mass": 180.16, "monoisotopic_mass": 180.042},
        "default_structure": {
            "smiles": "CC(=O)OC1=CC=CC=C1C(=O)O",
            "standard_inchi": "InChI=1S/C9H8O4/c1-6(10)13-8-5-3-2-4-7(8)9(11)12/h2-5H,1H3,(H,11,12)",
            "standard_inchi_key": "AAAAAAAAAAAAAA-UHFFFAOYSA-N"
        },
        "secondary_ids": ["CHEBI:1"],
        "database_accessions": {
            "REGISTRY NUMBER": [{"accession_number": "00-00-0", "source_name": "CAS", "url": "https://example.test/cas"}]
        },
        "ontology_relations": {
            "outgoing_relations": [
                {"relation_type": "is a", "init_id": "CHEBI:424242", "init_name": "synthetic-amide", "final_id": "CHEBI:100", "final_name": "amide"},
                {"relation_type": "has role", "init_id": "CHEBI:424242", "init_name": "synthetic-amide", "final_id": "CHEBI:200", "final_name": "tool compound"}
            ],
            "incoming_relations": [
                {"relation_type": "is a", "init_id": "CHEBI:300", "init_name": "child-amide", "final_id": "CHEBI:424242", "final_name": "synthetic-amide"}
            ]
        },
        "roles_classification": [{"chebi_accession": "CHEBI:200", "name": "tool compound", "definition": "invented role"}],
        "modified_on": "2026-01-01",
        "is_released": true
    })
}

fn rhea_sparql(body: &str) -> Response {
    let form = parse_form(body);
    let query = form.get("query").cloned().unwrap_or_default();
    if query.contains("COUNT") {
        let n = if query.contains("missing-compound") {
            "0"
        } else {
            "5"
        };
        return json_ok(json!({"results": {"bindings": [{"n": {"value": n}}]}}));
    }
    if query.contains("?p ?o") {
        if !query.contains("RHEA:424242") {
            return json_ok(json!({"results": {"bindings": []}}));
        }
        return json_ok(json!({"results": {"bindings": [
            {"p": {"value": "http://rdf.rhea-db.org/equation"}, "o": {"value": "synthetic-amide = synthetic-acid"}},
            {"p": {"value": "http://rdf.rhea-db.org/status"}, "o": {"value": "http://rdf.rhea-db.org/Approved"}},
            {"p": {"value": "http://rdf.rhea-db.org/isTransport"}, "o": {"value": "false"}},
            {"p": {"value": "http://rdf.rhea-db.org/isChemicallyBalanced"}, "o": {"value": "true"}},
            {"p": {"value": "http://rdf.rhea-db.org/ec"}, "o": {"value": "http://purl.uniprot.org/enzyme/2.1.1.160"}},
            {"p": {"value": "http://rdf.rhea-db.org/citation"}, "o": {"value": "http://www.ncbi.nlm.nih.gov/pubmed/11111111"}},
            {"p": {"value": "http://rdf.rhea-db.org/directionalReaction"}, "o": {"value": "http://rdf.rhea-db.org/424243"}},
            {"p": {"value": "http://rdf.rhea-db.org/bidirectionalReaction"}, "o": {"value": "http://rdf.rhea-db.org/424244"}}
        ]}}));
    }
    if query.contains("coefProp") {
        return json_ok(json!({"results": {"bindings": [
            {"side": {"value": "http://rdf.rhea-db.org/424242_L"}, "coefProp": {"value": "http://rdf.rhea-db.org/contains"}, "cacc": {"value": "CHEBI:424242"}, "cname": {"value": "synthetic-amide"}},
            {"side": {"value": "http://rdf.rhea-db.org/424242_R"}, "coefProp": {"value": "http://rdf.rhea-db.org/contains2"}, "cacc": {"value": "CHEBI:15377"}, "cname": {"value": "water"}}
        ]}}));
    }
    if query.contains("missing-compound") {
        return json_ok(json!({"results": {"bindings": []}}));
    }
    json_ok(json!({"results": {"bindings": [
        {"accession": {"value": "RHEA:424242"}, "equation": {"value": "synthetic-amide = synthetic-acid"}, "status": {"value": "http://rdf.rhea-db.org/Approved"}},
        {"accession": {"value": "RHEA:424250"}, "equation": {"value": "synthetic-amide + H2O = synthetic-acid"}, "status": {"value": "http://rdf.rhea-db.org/Preliminary"}}
    ]}}))
}

fn json_ok(value: Value) -> Response {
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        value.to_string(),
    )
        .into_response()
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        json!({"Fault": {"Code": "PUGREST.NotFound"}}).to_string(),
    )
        .into_response()
}

fn parse_form(body: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for part in body.split('&') {
        if part.is_empty() {
            continue;
        }
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        out.insert(pct_decode(key), pct_decode(&value.replace('+', " ")));
    }
    out
}

fn pct_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn assert_no_secret(value: &str) {
    assert!(!value.contains("operator@example.test"), "{value}");
    assert!(!value.contains("synthetic-key"), "{value}");
}

#[tokio::test]
async fn bio_call_covers_each_provider_with_source_urls() {
    let (bio, captured, server) = data_client().await;
    let pubchem = bio
        .call(
            "pubchem_search_compounds",
            &json!({"query": "synthetic-amide", "max_cids": 1}),
        )
        .await
        .unwrap();
    assert_eq!(pubchem["source"], "PubChem");
    assert_eq!(pubchem["n_cids_total"], 2);
    assert_eq!(pubchem["truncated"], true);
    assert_eq!(pubchem["cids"], json!([424242]));
    assert_eq!(
        pubchem["properties"][0]["url"],
        "https://pubchem.ncbi.nlm.nih.gov/compound/424242"
    );
    let request = captured.lock().unwrap().clone();
    assert!(
        request.contains("email=operator%40example.test")
            || request.contains("operator@example.test")
    );
    assert_no_secret(&pubchem.to_string());

    let chebi = bio
        .call("chebi_search", &json!({"term": "synthetic-amide"}))
        .await
        .unwrap();
    assert_eq!(chebi["source"], "ChEBI");
    assert_eq!(chebi["api_total"], 3);
    assert_eq!(
        chebi["results"][0]["url"],
        "https://www.ebi.ac.uk/chebi/searchId.do?chebiId=CHEBI:424242"
    );

    let rhea = bio
        .call(
            "rhea_search_reactions",
            &json!({"query": "synthetic-amide", "limit": 50}),
        )
        .await
        .unwrap();
    assert_eq!(rhea["source"], "Rhea");
    assert_eq!(rhea["query_type"], "text");
    assert_eq!(rhea["api_total"], 5);
    assert_eq!(rhea["truncated"], true);
    assert_eq!(
        rhea["reactions"][0]["url"],
        "https://www.rhea-db.org/rhea/424242"
    );

    let binding = bio
        .call(
            "bindingdb_ligands_by_target",
            &json!({"uniprot": "P42424", "max_rows": 1}),
        )
        .await
        .unwrap();
    assert_eq!(binding["source"], "BindingDB");
    assert_eq!(binding["n_rows_total"], 2);
    assert_eq!(binding["truncated"], true);
    assert_eq!(binding["rows"][0]["monomer_id"], "88");
    assert!(binding["rows"][0]["url"]
        .as_str()
        .unwrap()
        .contains("monomerid=88"));
    assert_no_secret(&binding.to_string());
    server.abort();
}

#[tokio::test]
async fn missing_ids_duplicates_and_empty_matches() {
    let (bio, _, server) = data_client().await;
    let compounds = bio
        .call(
            "pubchem_get_compounds",
            &json!({
                "cids": [424242, 999999, 424242],
                "include_synonyms": true,
                "max_synonyms": 1
            }),
        )
        .await
        .unwrap();
    assert_eq!(compounds["n_requested"], 3);
    assert_eq!(compounds["duplicates"], json!([424242]));
    assert_eq!(compounds["not_found"], json!([999999]));
    assert_eq!(compounds["records"].as_array().unwrap().len(), 1);
    assert_eq!(
        compounds["records"][0]["synonyms"],
        json!(["synthetic-amide"])
    );
    assert_eq!(compounds["records"][0]["n_synonyms_total"], 3);
    assert_eq!(compounds["records"][0]["synonyms_truncated"], true);

    let missing_search = bio
        .call(
            "pubchem_search_compounds",
            &json!({"query": "missing-compound", "with_properties": false}),
        )
        .await
        .unwrap();
    assert_eq!(missing_search["n_cids_total"], 0);
    assert_eq!(missing_search["cids"], json!([]));

    let missing_entity = bio
        .call("chebi_get_entity", &json!({"chebi_id": "CHEBI:1"}))
        .await
        .unwrap_err()
        .to_string();
    assert!(missing_entity.contains("no entity"), "{missing_entity}");

    let missing_rhea = bio
        .call("rhea_get_reaction", &json!({"rhea_id": "RHEA:1"}))
        .await
        .unwrap_err()
        .to_string();
    assert!(missing_rhea.contains("no reaction"), "{missing_rhea}");

    let empty_binding = bio
        .call("bindingdb_ligands_by_target", &json!({"uniprot": "P00000"}))
        .await
        .unwrap();
    assert_eq!(empty_binding["n_rows_total"], 0);
    server.abort();
}

#[tokio::test]
async fn remaining_tools_safety_assay_ontology_and_targets() {
    let (bio, _, server) = data_client().await;
    let safety = bio
        .call("pubchem_get_safety", &json!({"cid": 424242}))
        .await
        .unwrap();
    assert_eq!(safety["found"], true);
    assert_eq!(safety["ghs"]["signals"], json!(["Danger"]));
    assert_eq!(safety["ghs"]["pictograms"], json!(["Irritant"]));
    assert_eq!(
        safety["url"],
        "https://pubchem.ncbi.nlm.nih.gov/compound/424242"
    );

    let absent_safety = bio
        .call("pubchem_get_safety", &json!({"cid": 999999}))
        .await
        .unwrap();
    assert_eq!(absent_safety["found"], false);
    assert!(absent_safety["ghs"].is_null());

    let assay = bio
        .call(
            "pubchem_get_bioassay_summary",
            &json!({"cid": 424242, "active_only": true}),
        )
        .await
        .unwrap();
    assert_eq!(assay["n_rows_total"], 1);
    assert_eq!(assay["rows"][0]["Activity Outcome"], "Active");
    assert_eq!(
        assay["rows"][0]["url"],
        "https://pubchem.ncbi.nlm.nih.gov/bioassay/1"
    );

    let similar = bio
        .call(
            "pubchem_similarity_search",
            &json!({"smiles": "CCO", "max_records": 2, "with_properties": true}),
        )
        .await
        .unwrap();
    assert_eq!(similar["may_be_truncated"], true);
    assert_eq!(similar["properties"][0]["CID"], 424242);

    let entity = bio
        .call(
            "chebi_get_entity",
            &json!({"chebi_id": "424242", "max_synonyms": 1, "max_xrefs": 1}),
        )
        .await
        .unwrap();
    assert_eq!(entity["chebi_accession"], "CHEBI:424242");
    assert_eq!(entity["n_synonyms_total"], 2);
    assert_eq!(entity["synonyms_truncated"], true);
    assert!(entity.get("outgoing_relations").is_none());

    let ontology = bio
        .call(
            "chebi_get_ontology",
            &json!({"chebi_id": "CHEBI:424242", "relation_type": "is a"}),
        )
        .await
        .unwrap();
    assert_eq!(ontology["n_outgoing_total"], 1);
    assert_eq!(
        ontology["outgoing_relations"][0]["final_chebi_id"],
        "CHEBI:100"
    );
    assert_eq!(ontology["n_incoming_total"], 1);

    let reaction = bio
        .call("rhea_get_reaction", &json!({"rhea_id": "424242"}))
        .await
        .unwrap();
    assert_eq!(reaction["status"], "Approved");
    assert_eq!(reaction["ec_numbers"], json!(["2.1.1.160"]));
    assert_eq!(reaction["left_side"][0]["coefficient"], "1");
    assert_eq!(reaction["right_side"][0]["coefficient"], "2");

    let chebi_query = bio
        .call("rhea_search_reactions", &json!({"query": "CHEBI:424242"}))
        .await
        .unwrap();
    assert_eq!(chebi_query["query_type"], "chebi");

    let targets = bio
        .call("bindingdb_targets_by_compound", &json!({"smiles": "CCO"}))
        .await
        .unwrap();
    assert_eq!(targets["api_hit_count"], 1);
    assert_eq!(targets["rows"][0]["target_name"], "synthetic-kinase");
    assert_no_secret(&targets.to_string());
    server.abort();
}

#[tokio::test]
async fn upstream_429_and_malformed_json_do_not_echo_secrets() {
    let cases = [
        ("pubchem_get_safety", json!({"cid": 424242}), "PubChem"),
        ("chebi_search", json!({"term": "synthetic-amide"}), "EBI"),
        (
            "rhea_get_reaction",
            json!({"rhea_id": "RHEA:424242"}),
            "Rhea",
        ),
        (
            "bindingdb_ligands_by_target",
            json!({"uniprot": "P42424"}),
            "BindingDB",
        ),
    ];
    for (name, args, source) in cases {
        let (bio, _, server) = status_client(429, "operator@example.test").await;
        let error = bio.call(name, &args).await.unwrap_err().to_string();
        server.abort();
        assert!(error.contains("HTTP 429"), "{name}: {error}");
        assert!(error.contains(source), "{name}: {error}");
        assert_no_secret(&error);
    }
    for (name, args) in [
        ("pubchem_get_safety", json!({"cid": 424242})),
        ("chebi_search", json!({"term": "synthetic-amide"})),
        ("rhea_get_reaction", json!({"rhea_id": "RHEA:424242"})),
        ("bindingdb_ligands_by_target", json!({"uniprot": "P42424"})),
    ] {
        let (bio, _, server) = status_client(200, "{").await;
        let error = bio.call(name, &args).await.unwrap_err().to_string();
        server.abort();
        assert!(error.contains("invalid JSON"), "{name}: {error}");
        assert_no_secret(&error);
    }
}

#[tokio::test]
async fn empty_bindingdb_body_is_no_hits_not_an_error() {
    let (bio, _, server) = status_client(200, "   ").await;
    let result = bio
        .call("bindingdb_targets_by_compound", &json!({"smiles": "CCO"}))
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["n_rows_total"], 0);
    assert_eq!(result["source"], "BindingDB");
}
