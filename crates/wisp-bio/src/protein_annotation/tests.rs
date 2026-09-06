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
    NativeBio::test_client(
        &[
            (
                "INTERPRO_BASE_URL".into(),
                base.trim_end_matches('/').into(),
            ),
            (
                "PROTEIN_ATLAS_BASE_URL".into(),
                base.trim_end_matches('/').into(),
            ),
            ("STRING_BASE_URL".into(), base.trim_end_matches('/').into()),
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

fn architecture_row() -> Value {
    json!({
        "metadata": {
            "accession": "IPR000719",
            "name": "Protein kinase domain",
            "source_database": "interpro",
            "type": "domain",
            "member_databases": {"pfam": {"PF00069": "Protein kinase domain"}},
            "go_terms": [{"identifier": "GO:0004672", "name": "protein kinase activity", "category": {"code": "F"}}]
        },
        "proteins": [{
            "accession": "p04637",
            "protein_length": 393,
            "entry_protein_locations": [{
                "fragments": [{"start": 191, "end": 312, "dc-status": "CONTINUOUS"}],
                "representative": false,
                "model": null,
                "score": null
            }]
        }]
    })
}

fn provider_app(captured: Arc<StdMutex<Vec<String>>>) -> Router {
    let log = captured.clone();
    Router::new()
        .route(
            "/entry/interpro/protein/uniprot/{acc}/",
            get(|Path(acc): Path<String>| async move {
                if acc.eq_ignore_ascii_case("P00000") {
                    return StatusCode::NOT_FOUND.into_response();
                }
                if acc.eq_ignore_ascii_case("P99999") {
                    return StatusCode::NO_CONTENT.into_response();
                }
                axum::Json(json!({
                    "count": 1,
                    "next": null,
                    "results": [architecture_row()]
                }))
                .into_response()
            }),
        )
        .route(
            "/entry/interpro/",
            get(|uri: axum::http::Uri| async move {
                let query = uri.query().unwrap_or("");
                if query.contains("search=nomatch") {
                    return StatusCode::NO_CONTENT.into_response();
                }
                if query.contains("cursor=page2") {
                    return axum::Json(json!({
                        "count": 2,
                        "next": null,
                        "results": [{"metadata": {
                            "accession": "IPR000002",
                            "name": "Second",
                            "type": "family",
                            "source_database": "interpro"
                        }}]
                    }))
                    .into_response();
                }
                axum::Json(json!({
                    "count": 2,
                    "next": "/entry/interpro/?search=kinase&cursor=page2",
                    "results": [{"metadata": {
                        "accession": "IPR000001",
                        "name": "First",
                        "type": "domain",
                        "source_database": "interpro"
                    }}]
                }))
                .into_response()
            }),
        )
        .route(
            "/entry/interpro/{acc}/",
            get(|Path(acc): Path<String>| async move {
                if acc == "IPR999999" {
                    return StatusCode::NOT_FOUND.into_response();
                }
                axum::Json(json!({
                    "metadata": {
                        "accession": acc,
                        "name": {"name": "Protein kinase domain", "short": "Pkinase"},
                        "type": "domain",
                        "source_database": "interpro",
                        "member_databases": {"pfam": {"PF00069": "Protein kinase domain"}},
                        "go_terms": [{"identifier": "GO:0004672", "name": "protein kinase activity"}]
                    }
                }))
                .into_response()
            }),
        )
        .route(
            "/entry/pfam/{acc}/",
            get(|Path(acc): Path<String>| async move {
                axum::Json(json!({
                    "metadata": {
                        "accession": acc,
                        "name": "Protein kinase domain",
                        "type": "domain",
                        "source_database": "pfam",
                        "integrated": "IPR000719",
                        "set_info": {"accession": "CL0016", "name": "Pkinase"}
                    }
                }))
                .into_response()
            }),
        )
        .route(
            "/set/pfam/",
            get(|Query(params): Query<HashMap<String, String>>| async move {
                if params.get("search").map(String::as_str) == Some("emptyclan") {
                    return StatusCode::NO_CONTENT.into_response();
                }
                axum::Json(json!({
                    "count": 1,
                    "next": null,
                    "results": [{"metadata": {
                        "accession": "CL0016",
                        "name": "Pkinase",
                        "source_database": "pfam"
                    }}]
                }))
                .into_response()
            }),
        )
        .route(
            "/set/pfam/{acc}/",
            get(|Path(acc): Path<String>| async move {
                axum::Json(json!({
                    "metadata": {
                        "accession": acc,
                        "name": "Pkinase",
                        "source_database": "pfam",
                        "relationships": {"nodes": [
                            {"accession": "PF00069", "name": "Pkinase", "short_name": "Pkinase", "type": "family"},
                            {"accession": "PF07714", "name": "Pkinase_Tyr", "short_name": "Pkinase_Tyr", "type": "family"}
                        ]}
                    }
                }))
                .into_response()
            }),
        )
        .route(
            "/protein/uniprot/entry/pfam/{acc}/",
            get(
                |Path(_acc): Path<String>, Query(params): Query<HashMap<String, String>>| async move {
                    if params.get("page_size").map(String::as_str) == Some("1") {
                        return axum::Json(json!({"count": 128, "next": null, "results": []}))
                            .into_response();
                    }
                    axum::Json(json!({
                        "count": 128,
                        "next": null,
                        "results": [{
                            "metadata": {
                                "accession": "P04637",
                                "name": "Cellular tumor antigen p53",
                                "source_database": "reviewed",
                                "length": 393,
                                "source_organism": {"taxId": "9606", "scientificName": "Homo sapiens"}
                            }
                        }]
                    }))
                    .into_response()
                },
            ),
        )
        .route(
            "/proteome/uniprot/entry/pfam/{acc}/",
            get(
                |Path(_acc): Path<String>, Query(params): Query<HashMap<String, String>>| async move {
                    let results = if params.get("page_size").map(String::as_str) == Some("1") {
                        json!([])
                    } else {
                        json!([{
                            "metadata": {
                                "accession": "UP000005640",
                                "name": "Homo sapiens",
                                "is_reference": true,
                                "taxonomy": "9606"
                            }
                        }])
                    };
                    axum::Json(json!({"count": 42, "next": null, "results": results})).into_response()
                },
            ),
        )
        .route(
            "/api/search_download.php",
            get(|Query(params): Query<HashMap<String, String>>| async move {
                let q = params.get("search").cloned().unwrap_or_default();
                if q == "none-such" {
                    return axum::Json(json!([])).into_response();
                }
                if q == "AMBIG" {
                    return axum::Json(json!([
                        {"Gene": "AMBIG", "Ensembl": "ENSG00000000001"},
                        {"Gene": "AMBIG", "Ensembl": "ENSG00000000002"}
                    ]))
                    .into_response();
                }
                axum::Json(json!([{
                    "Gene": "TP53",
                    "Gene synonym": ["P53"],
                    "Ensembl": "ENSG00000141510",
                    "Gene description": "tumor protein p53",
                    "Uniprot": ["P04637"]
                }]))
                .into_response()
            }),
        )
        .route(
            "/{file}",
            get(|Path(file): Path<String>| async move {
                let Some(ensg) = file.strip_suffix(".json") else {
                    return StatusCode::NOT_FOUND.into_response();
                };
                if ensg == "ENSG00000141510" {
                    return axum::Json(json!({
                        "Gene": "TP53",
                        "Ensembl": "ENSG00000141510",
                        "Gene description": "tumor protein p53",
                        "Uniprot": ["P04637"],
                        "RNA tissue specificity": "low tissue specificity",
                        "Subcellular location": ["Nucleoplasm"],
                        "Antibody": ["CAB000000"],
                        "Cancer prognostics - Breast Invasive Carcinoma (TCGA)": "unprognostic"
                    }))
                    .into_response();
                }
                StatusCode::NOT_FOUND.into_response()
            }),
        )
        .route(
            "/json/version",
            post({
                let log = log.clone();
                move |body: String| {
                    let log = log.clone();
                    async move {
                        log.lock().unwrap().push(format!("version {body}"));
                        axum::Json(json!([{
                            "string_version": "12.0",
                            "string_stable_address": "https://version-12-0.string-db.org"
                        }]))
                    }
                }
            }),
        )
        .route(
            "/json/get_string_ids",
            post({
                let log = log.clone();
                move |body: String| {
                    let log = log.clone();
                    async move {
                        log.lock().unwrap().push(format!("ids {body}"));
                        if body.contains("NOPE") && !body.contains("TP53") {
                            return StatusCode::NOT_FOUND.into_response();
                        }
                        let mut rows = Vec::new();
                        if body.contains("TP53") {
                            rows.push(json!({
                                "queryIndex": 0,
                                "queryItem": "TP53",
                                "stringId": "9606.ENSP00000269305",
                                "ncbiTaxonId": 9606,
                                "preferredName": "TP53",
                                "annotation": "cellular tumor antigen p53"
                            }));
                        }
                        if body.contains("BRCA1") {
                            let idx = if body.contains("TP53") { 1 } else { 0 };
                            rows.push(json!({
                                "queryIndex": idx,
                                "queryItem": "BRCA1",
                                "stringId": "9606.ENSP00000350283",
                                "ncbiTaxonId": 9606,
                                "preferredName": "BRCA1",
                                "annotation": "breast cancer type 1"
                            }));
                        }
                        axum::Json(Value::Array(rows)).into_response()
                    }
                }
            }),
        )
        .route(
            "/json/network",
            post({
                let log = log.clone();
                move |body: String| {
                    let log = log.clone();
                    async move {
                        log.lock().unwrap().push(format!("network {body}"));
                        axum::Json(json!([{
                            "stringId_A": "9606.ENSP00000269305",
                            "stringId_B": "9606.ENSP00000350283",
                            "preferredName_A": "TP53",
                            "preferredName_B": "BRCA1",
                            "ncbiTaxonId": 9606,
                            "score": 0.9,
                            "nscore": 0.0,
                            "fscore": 0.0,
                            "pscore": 0.0,
                            "ascore": 0.0,
                            "escore": 0.8,
                            "dscore": 0.5,
                            "tscore": 0.4
                        }]))
                    }
                }
            }),
        )
        .route(
            "/json/homology",
            post(|| async {
                axum::Json(json!([
                    {
                        "ncbiTaxonId_A": 9606,
                        "stringId_A": "9606.ENSP00000269305",
                        "ncbiTaxonId_B": 9606,
                        "stringId_B": "9606.ENSP00000350283",
                        "bitscore": "120.5"
                    },
                    {
                        "ncbiTaxonId_A": 9606,
                        "stringId_A": "9606.ENSP00000269305",
                        "ncbiTaxonId_B": 9606,
                        "stringId_B": "9606.ENSP00000269305",
                        "bitscore": 406.8
                    }
                ]))
            }),
        )
        .route(
            "/json/homology_best",
            post(|| async {
                axum::Json(json!([{
                    "ncbiTaxonId_A": 9606,
                    "stringId_A": "9606.ENSP00000269305",
                    "ncbiTaxonId_B": 10090,
                    "stringId_B": "10090.ENSMUSP00000000001",
                    "bitscore": 598.2
                }]))
            }),
        )
}

#[test]
fn catalog_registers_thirteen_protein_annotation_tools() {
    let names: Vec<_> = catalog()
        .into_iter()
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        names,
        vec![
            ("protein-annotation", "get_domain_architecture".into()),
            ("protein-annotation", "get_interpro_entry".into()),
            ("protein-annotation", "get_pfam_clan".into()),
            ("protein-annotation", "get_pfam_family_proteins".into()),
            ("protein-annotation", "get_pfam_family_proteomes".into()),
            ("protein-annotation", "get_protein_atlas_gene".into()),
            (
                "protein-annotation",
                "get_string_best_similarity_hits".into()
            ),
            ("protein-annotation", "get_string_network".into()),
            ("protein-annotation", "get_string_similarity_scores".into()),
            ("protein-annotation", "map_string_ids".into()),
            ("protein-annotation", "search_interpro_entries".into()),
            ("protein-annotation", "search_pfam_clans".into()),
            ("protein-annotation", "search_protein_atlas".into()),
        ]
    );
    assert!(crate::contains_tool("get_domain_architecture"));
    assert_eq!(
        crate::domain_for_tool("map_string_ids"),
        Some("protein-annotation")
    );
    assert!(crate::package_selects(
        "mcp_protein_annotation",
        "protein-annotation"
    ));
    assert!(crate::selected_by_package("mcp_protein_annotation"));
}

#[test]
fn rejects_unbounded_or_malformed_arguments() {
    assert!(interpro::is_uniprot("P04637"));
    assert!(interpro::is_uniprot("A0A0A0ABC1"));
    assert!(!interpro::is_uniprot("P46"));
    assert!(protein_atlas::is_ensg("ENSG00000141510"));
    assert!(!protein_atlas::is_ensg("TP53"));
    for args in [
        json!({"accessions": []}),
        json!({"accessions": ["P04637,P53"]}),
        json!({"accessions": ["P04637"], "max_results": 0}),
        json!({"accessions": ["P04637"], "max_results": 201}),
        json!({"accessions": vec!["P04637"; 21]}),
    ] {
        match serde_json::from_value::<interpro::DomainArgs>(args.clone()) {
            Ok(parsed) => assert!(
                require_ids(&parsed.accessions, MAX_PROTEINS, "UniProt accession").is_err()
                    || bound_page(parsed.max_results).is_err()
                    || parsed
                        .accessions
                        .iter()
                        .any(|id| !interpro::is_uniprot(id) || id.contains(',')),
                "{args}"
            ),
            Err(_) => {}
        }
    }
    assert!(serde_json::from_value::<interpro::DomainArgs>(
        json!({"accessions": ["P04637"], "api_key": "secret"})
    )
    .is_err());
    assert!(serde_json::from_value::<interpro::SearchEntries>(
        json!({"query": "kinase", "token": "x"})
    )
    .is_err());
    assert!(serde_json::from_value::<protein_atlas::GeneArgs>(
        json!({"gene": "TP53", "api_key": "x"})
    )
    .is_err());
    assert!(serde_json::from_value::<string::MapArgs>(
        json!({"symbols": ["TP53"], "api_key": "x"})
    )
    .is_err());
    assert!(require_ids(&["TP53,BRCA1".into()], MAX_IDS, "symbol").is_err());
    assert!(bound_page(0).is_err());
    assert!(taxon_id(0, "species").is_err());
}

#[tokio::test]
async fn interpro_tools_report_missing_ids_source_urls_and_pagination() {
    let (bio, server) = serve(provider_app(Arc::new(StdMutex::new(Vec::new())))).await;
    let architecture = bio
        .call(
            "get_domain_architecture",
            &json!({"accessions": ["P04637", "P00000", "P99999"], "max_results": 50}),
        )
        .await
        .unwrap();
    let search = bio
        .call(
            "search_interpro_entries",
            &json!({"query": "kinase", "entry_type": "domain", "max_results": 1}),
        )
        .await
        .unwrap();
    let empty = bio
        .call("search_interpro_entries", &json!({"query": "nomatch"}))
        .await
        .unwrap();
    let entry = bio
        .call("get_interpro_entry", &json!({"accession": "IPR000719"}))
        .await
        .unwrap();
    let pfam = bio
        .call("get_interpro_entry", &json!({"accession": "PF00069"}))
        .await
        .unwrap();
    let clan = bio
        .call(
            "get_pfam_clan",
            &json!({"clan_accession": "CL0016", "max_results": 1}),
        )
        .await
        .unwrap();
    let proteins = bio
        .call(
            "get_pfam_family_proteins",
            &json!({"pfam_accession": "PF00069", "count_only": true}),
        )
        .await
        .unwrap();
    let members = bio
        .call(
            "get_pfam_family_proteins",
            &json!({"pfam_accession": "PF00069", "max_results": 25}),
        )
        .await
        .unwrap();
    let proteomes = bio
        .call(
            "get_pfam_family_proteomes",
            &json!({"pfam_accession": "PF00069"}),
        )
        .await
        .unwrap();
    let clans = bio
        .call("search_pfam_clans", &json!({"query": "kinase"}))
        .await
        .unwrap();
    let missing_entry = bio
        .call("get_interpro_entry", &json!({"accession": "IPR999999"}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();

    assert_eq!(architecture["source"], "InterPro");
    assert_eq!(architecture["missing_ids"], json!(["P00000"]));
    assert_eq!(architecture["returned"], 2);
    assert_eq!(architecture["proteins"][0]["accession"], "P04637");
    assert_eq!(architecture["proteins"][0]["protein_length"], 393);
    assert_eq!(
        architecture["proteins"][0]["url"],
        "https://www.ebi.ac.uk/interpro/protein/UniProt/P04637/"
    );
    assert_eq!(
        architecture["proteins"][0]["entries"][0]["accession"],
        "IPR000719"
    );
    assert_eq!(architecture["proteins"][1]["total_entries"], 0);
    assert_eq!(search["total"], 2);
    assert_eq!(search["returned"], 1);
    assert_eq!(search["has_more"], true);
    assert_eq!(search["results"][0]["accession"], "IPR000001");
    assert_eq!(empty["total"], 0);
    assert_eq!(empty["has_more"], false);
    assert_eq!(
        entry["source_url"],
        "https://www.ebi.ac.uk/interpro/entry/InterPro/IPR000719/"
    );
    assert_eq!(entry["name"]["short"], "Pkinase");
    assert_eq!(pfam["integrated"], "IPR000719");
    assert_eq!(
        pfam["source_url"],
        "https://www.ebi.ac.uk/interpro/entry/pfam/PF00069/"
    );
    assert_eq!(clan["member_count"], 2);
    assert_eq!(clan["returned"], 1);
    assert_eq!(clan["has_more"], true);
    assert_eq!(proteins["count_only"], true);
    assert_eq!(proteins["total"], 128);
    assert_eq!(proteins["results"], json!([]));
    assert_eq!(members["results"][0]["accession"], "P04637");
    assert_eq!(members["has_more"], true);
    assert_eq!(proteomes["count_only"], true);
    assert_eq!(proteomes["total"], 42);
    assert_eq!(clans["results"][0]["accession"], "CL0016");
    assert!(missing_entry.contains("not found"), "{missing_entry}");
}

#[tokio::test]
async fn protein_atlas_and_string_tools_dispatch_through_native_bio_call() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let (bio, server) = serve(provider_app(captured.clone())).await;
    let gene = bio
        .call("get_protein_atlas_gene", &json!({"gene": "TP53"}))
        .await
        .unwrap();
    let ensg = bio
        .call(
            "get_protein_atlas_gene",
            &json!({"gene": "ENSG00000141510", "full": true}),
        )
        .await
        .unwrap();
    let search = bio
        .call(
            "search_protein_atlas",
            &json!({"query": "p53", "max_results": 1}),
        )
        .await
        .unwrap();
    let mapped = bio
        .call(
            "map_string_ids",
            &json!({"symbols": ["TP53", "MISSING"], "species": 9606}),
        )
        .await
        .unwrap();
    let unmapped = bio
        .call("map_string_ids", &json!({"symbols": ["NOPE"]}))
        .await
        .unwrap();
    let network = bio
        .call(
            "get_string_network",
            &json!({"symbols": ["TP53", "BRCA1"], "required_score": 700, "max_results": 10}),
        )
        .await
        .unwrap();
    let homology = bio
        .call(
            "get_string_similarity_scores",
            &json!({"symbols": ["TP53", "BRCA1"]}),
        )
        .await
        .unwrap();
    let hits = bio
        .call(
            "get_string_best_similarity_hits",
            &json!({"symbols": ["TP53"], "target_species": 10090}),
        )
        .await
        .unwrap();
    let ambiguous = bio
        .call("get_protein_atlas_gene", &json!({"gene": "AMBIG"}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();

    assert_eq!(gene["source"], "Human Protein Atlas");
    assert_eq!(gene["ensembl"], "ENSG00000141510");
    assert_eq!(
        gene["source_url"],
        "https://www.proteinatlas.org/ENSG00000141510"
    );
    assert_eq!(gene["summary"]["identity"]["Gene"], "TP53");
    assert_eq!(
        gene["summary"]["pathology"]["prognostics"]["Breast Invasive Carcinoma (TCGA)"],
        "unprognostic"
    );
    assert_eq!(ensg["full"], true);
    assert_eq!(ensg["record"]["Gene"], "TP53");
    assert_eq!(search["returned"], 1);
    assert_eq!(search["results"][0]["Ensembl"], "ENSG00000141510");
    assert_eq!(mapped["mapped"][0]["string_id"], "9606.ENSP00000269305");
    assert_eq!(mapped["unmapped"], json!(["MISSING"]));
    assert_eq!(
        mapped["mapped"][0]["url"],
        "https://version-12-0.string-db.org/network/9606.ENSP00000269305"
    );
    assert_eq!(unmapped["mapped"], json!([]));
    assert_eq!(unmapped["unmapped"], json!(["NOPE"]));
    assert_eq!(network["query"]["add_nodes"], 0);
    assert_eq!(network["query"]["required_score"], 700);
    assert_eq!(network["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(network["edges"][0]["evidence"]["escore"], 0.8);
    assert!(network["edges"][0]["evidence"].get("nscore").is_none());
    assert_eq!(homology["pairs"].as_array().unwrap().len(), 2);
    assert_eq!(homology["pairs"][0]["self"], true);
    assert_eq!(homology["pairs"][1]["self"], false);
    assert_eq!(hits["hits"][0]["hit_taxon"], 10090);
    assert!(ambiguous.contains("multiple"));

    let traffic = captured.lock().unwrap().join("\n");
    assert!(
        traffic.contains("caller_identity=wisp-science"),
        "{traffic}"
    );
    assert!(traffic.contains("echo_query=1"), "{traffic}");
    assert!(traffic.contains("add_nodes=0"), "{traffic}");
    assert!(traffic.contains("required_score=700"), "{traffic}");
}

#[tokio::test]
async fn rejects_rate_limits_malformed_json_and_oversized_bodies() {
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
        (
            StatusCode::OK,
            "<!doctype html><html><body>app</body></html>".into(),
            "HTML",
        ),
    ] {
        let app = Router::new().route(
            "/entry/interpro/",
            get({
                let body = body.clone();
                move || {
                    let body = body.clone();
                    async move { (status, [("retry-after", "60")], body).into_response() }
                }
            }),
        );
        let (bio, server) = serve(app).await;
        let error = bio
            .call("search_interpro_entries", &json!({"query": "kinase"}))
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
async fn string_rate_limit_does_not_echo_secrets() {
    let app = Router::new().route(
        "/json/version",
        post(|| async {
            (
                StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", "60")],
                "secret-token",
            )
                .into_response()
        }),
    );
    let (bio, server) = serve(app).await;
    let error = bio
        .call("map_string_ids", &json!({"symbols": ["TP53"]}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("HTTP 429"), "{error}");
    assert!(!error.contains("secret-token"), "{error}");
}

#[tokio::test]
async fn unknown_tool_name_is_rejected() {
    let (bio, server) = serve(Router::new()).await;
    let error = call(&bio, "not_a_protein_tool", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("unknown native biological tool"));
}
