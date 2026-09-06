use super::*;
use crate::http::{Http, MAX_RESPONSE};
use crate::NativeBio;
use axum::{
    extract::Path,
    http::{StatusCode, Uri},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde_json::json;
use std::sync::{Arc, Mutex as StdMutex};

fn test_bio(base: &str) -> NativeBio {
    let base = base.trim_end_matches('/').to_string();
    NativeBio::test_client(
        &[
            ("MYGENE_BASE_URL".into(), base.clone()),
            ("UNIPROT_BASE_URL".into(), base.clone()),
            ("OLS_BASE_URL".into(), base.clone()),
            ("QUICKGO_BASE_URL".into(), base.clone()),
            ("REACTOME_BASE_URL".into(), base),
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

#[test]
fn catalog_registers_seven_genes_ontologies_tools_and_skips_kegg() {
    let names: Vec<_> = catalog()
        .into_iter()
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        names,
        vec![
            ("genes-ontologies", "get_go_annotations".into()),
            ("genes-ontologies", "get_ontology_term".into()),
            ("genes-ontologies", "get_uniprot_entries".into()),
            ("genes-ontologies", "list_ontologies".into()),
            ("genes-ontologies", "map_reactome_pathways".into()),
            ("genes-ontologies", "query_genes".into()),
            ("genes-ontologies", "search_ontology_terms".into()),
        ]
    );
    for kegg in ["get_kegg_entries", "search_kegg", "link_kegg_ids"] {
        assert!(!crate::contains_tool(kegg), "{kegg}");
        assert_eq!(crate::domain_for_tool(kegg), None);
    }
    assert_eq!(
        crate::domain_for_tool("query_genes"),
        Some("genes-ontologies")
    );
    assert!(crate::package_selects(
        "mcp_genes_ontologies",
        "genes-ontologies"
    ));
    assert!(crate::selected_by_package("mcp_genes_ontologies"));
}

#[test]
fn rejects_unbounded_or_malformed_arguments() {
    for args in [
        json!({"terms": []}),
        json!({"terms": ["TP53,BRCA1"]}),
        json!({"terms": [" "]}),
        json!({"terms": vec!["G".to_string(); 201]}),
        json!({"terms": ["TP53"], "api_key": "secret"}),
    ] {
        assert!(
            serde_json::from_value::<QueryGenes>(args.clone())
                .ok()
                .and_then(|parsed| require_terms(&parsed.terms, MAX_GENE_TERMS, "gene term").ok())
                .is_none(),
            "{args}"
        );
    }
    assert!(parse_uniprot_one("not-an-accession").is_err());
    assert!(parse_uniprot_one("P04637").is_ok());
    assert_eq!(parse_uniprot_one("uniprotkb:p04637").unwrap(), "P04637");
    assert!(is_uniprot_accession("A0A0A0MRZ8"));
    assert!(!is_uniprot_accession("GO:0008150"));
    assert!(evidence_codes("IDA").is_err());
    assert!(evidence_codes("ECO:0000314").is_ok());
    assert!(require_ontology_id("GO").is_ok());
    assert!(require_ontology_id("../etc").is_err());
    assert!(bound_u32(0, 1, 100, "max_results").is_err());
    assert!(bound_u32(101, 1, 100, "max_results").is_err());
}

#[tokio::test]
async fn query_genes_batches_terms_and_reports_missing_and_source_urls() {
    let captured = Arc::new(StdMutex::new(String::new()));
    let body = captured.clone();
    let app = Router::new().route(
        "/v3/query",
        post(move |incoming: String| {
            *body.lock().unwrap() = incoming;
            async {
                axum::Json(json!([
                    {
                        "query": "TP53",
                        "_id": "7157",
                        "symbol": "TP53",
                        "name": "tumor protein p53",
                        "taxid": 9606,
                        "entrezgene": 7157
                    },
                    {"query": "NOGENE", "notfound": true}
                ]))
            }
        }),
    );
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "query_genes",
            &json!({"terms": ["TP53", "NOGENE"], "scopes": "symbol", "species": "human"}),
        )
        .await
        .unwrap();
    server.abort();
    let form = captured.lock().unwrap().clone();
    assert!(form.contains("q=TP53"), "{form}");
    assert!(form.contains("NOGENE"), "{form}");
    assert!(form.contains("scopes=symbol"), "{form}");
    assert_eq!(result["source"], "MyGene.info");
    assert_eq!(result["source_url"], "https://mygene.info/v3/query");
    assert_eq!(result["n_input"], 2);
    assert_eq!(result["returned"], 1);
    assert_eq!(result["missing_terms"], json!(["NOGENE"]));
    assert_eq!(
        result["records"][0]["url"],
        "https://mygene.info/v3/gene/7157"
    );
}

#[tokio::test]
async fn uniprot_fields_fasta_and_txt_report_missing_accessions() {
    let seen = Arc::new(StdMutex::new(Vec::<String>::new()));
    let traffic = seen.clone();
    let app = Router::new().route(
        "/uniprotkb/search",
        get(move |uri: Uri| {
            let traffic = traffic.clone();
            async move {
                traffic.lock().unwrap().push(uri.to_string());
                let query = uri.query().unwrap_or("");
                if query.contains("format=tsv") {
                    (
                        StatusCode::OK,
                        "Entry\tEntry Name\tProtein names\nP04637\tP53_HUMAN\tsynthetic p53\n",
                    )
                        .into_response()
                } else if query.contains("format=fasta") {
                    (StatusCode::OK, ">sp|P04637|P53_HUMAN synthetic\nMEEPQ\n").into_response()
                } else {
                    (
                        StatusCode::OK,
                        "ID   P53_HUMAN\nAC   P04637;\nDE   RecName: Full=synthetic p53;\n//\n",
                    )
                        .into_response()
                }
            }
        }),
    );
    let (bio, server) = serve(app).await;
    let fields = bio
        .call(
            "get_uniprot_entries",
            &json!({
                "accessions": ["P04637", "P99999"],
                "fields": ["accession", "id", "protein_name"]
            }),
        )
        .await
        .unwrap();
    let fasta = bio
        .call(
            "get_uniprot_entries",
            &json!({"accessions": ["P04637", "P99999"], "format": "fasta"}),
        )
        .await
        .unwrap();
    let txt = bio
        .call(
            "get_uniprot_entries",
            &json!({"accessions": ["UniProtKB:P04637"], "format": "txt"}),
        )
        .await
        .unwrap();
    server.abort();
    let urls = seen.lock().unwrap().join("\n");
    assert!(urls.contains("format=tsv"), "{urls}");
    assert!(urls.contains("P04637"), "{urls}");
    assert!(urls.contains("accession_id"), "{urls}");
    assert_eq!(fields["source"], "UniProt");
    assert_eq!(
        fields["source_url"],
        "https://rest.uniprot.org/uniprotkb/search"
    );
    assert_eq!(fields["missing"], json!(["P99999"]));
    assert_eq!(
        fields["records"][0]["url"],
        "https://www.uniprot.org/uniprotkb/P04637"
    );
    assert_eq!(fields["records"][0]["protein_name"], "synthetic p53");
    assert_eq!(fasta["n_found"], 1);
    assert_eq!(fasta["missing"], json!(["P99999"]));
    assert!(fasta["records"]["P04637"]
        .as_str()
        .unwrap()
        .contains("MEEPQ"));
    assert_eq!(txt["n_found"], 1);
    assert_eq!(
        txt["urls"]["P04637"],
        "https://www.uniprot.org/uniprotkb/P04637"
    );
}

#[tokio::test]
async fn go_annotations_are_capped_and_include_source_urls() {
    let seen = Arc::new(StdMutex::new(Vec::<String>::new()));
    let traffic = seen.clone();
    let app = Router::new()
        .route(
            "/services/annotation/search",
            get(move |uri: Uri| {
                let traffic = traffic.clone();
                async move {
                    traffic.lock().unwrap().push(uri.to_string());
                    let page = uri
                        .query()
                        .unwrap_or("")
                        .split('&')
                        .find_map(|part| part.strip_prefix("page="))
                        .unwrap_or("1");
                    let go_id = if page == "1" {
                        "GO:0006915"
                    } else {
                        "GO:0008285"
                    };
                    axum::Json(json!({
                        "numberOfHits": 3,
                        "results": [{
                            "goId": go_id,
                            "goName": null,
                            "goAspect": "biological_process",
                            "goEvidence": "IDA",
                            "evidenceCode": "ECO:0000314",
                            "qualifier": "involved_in",
                            "reference": "PMID:1",
                            "assignedBy": "UniProt",
                            "geneProductId": "UniProtKB:P04637",
                            "symbol": "TP53",
                            "taxonId": 9606,
                            "date": "20200101"
                        }],
                        "pageInfo": {"resultsPerPage": 1, "current": page.parse::<u32>().unwrap_or(1), "total": 3}
                    }))
                }
            }),
        )
        .route(
            "/services/ontology/go/terms/{ids}",
            get(|Path(ids): Path<String>| async move {
                let mut results = Vec::new();
                for id in ids.replace("%3A", ":").split(',') {
                    results.push(json!({
                        "id": id,
                        "name": format!("synthetic {id}"),
                        "isObsolete": false,
                        "aspect": "biological_process"
                    }));
                }
                axum::Json(json!({"results": results}))
            }),
        );
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "get_go_annotations",
            &json!({
                "uniprot_accession": "P04637",
                "aspect": "biological_process",
                "evidence": "experimental_manual",
                "max_records": 2
            }),
        )
        .await
        .unwrap();
    server.abort();
    let urls = seen.lock().unwrap().join("\n");
    assert!(urls.contains("geneProductId=P04637"), "{urls}");
    assert!(urls.contains("aspect=biological_process"), "{urls}");
    assert!(urls.contains("evidenceCode=ECO"), "{urls}");
    assert_eq!(result["source"], "QuickGO");
    assert_eq!(result["total_annotations"], 3);
    assert_eq!(result["returned"], 2);
    assert_eq!(result["truncated"], true);
    assert_eq!(
        result["gene_product_url"],
        "https://www.ebi.ac.uk/QuickGO/annotations?geneProductId=P04637"
    );
    assert_eq!(
        result["records"][0]["url"],
        "https://www.ebi.ac.uk/QuickGO/term/GO:0006915"
    );
    assert_eq!(result["records"][0]["go_name"], "synthetic GO:0006915");
}

#[tokio::test]
async fn ols_list_search_and_term_dispatch() {
    let seen = Arc::new(StdMutex::new(Vec::<String>::new()));
    let traffic = seen.clone();
    let capture = move |uri: Uri| {
        let traffic = traffic.clone();
        async move {
            traffic.lock().unwrap().push(uri.to_string());
            let path = uri.path();
            if path == "/api/ontologies" {
                axum::Json(json!({
                    "_embedded": {"ontologies": [{
                        "ontologyId": "go",
                        "status": "LOADED",
                        "numberOfTerms": 12,
                        "numberOfProperties": 1,
                        "config": {"title": "Gene Ontology", "version": "synthetic", "description": "test ontology"}
                    }]},
                    "page": {"size": 50, "totalElements": 1, "totalPages": 1, "number": 0}
                }))
                .into_response()
            } else if path == "/api/ontologies/go" {
                axum::Json(json!({
                    "ontologyId": "go",
                    "status": "LOADED",
                    "numberOfTerms": 12,
                    "config": {"title": "Gene Ontology"}
                }))
                .into_response()
            } else if path == "/api/ontologies/missing" {
                StatusCode::NOT_FOUND.into_response()
            } else if path == "/api/search" {
                axum::Json(json!({
                    "response": {
                        "numFound": 8,
                        "start": 0,
                        "docs": [{
                            "obo_id": "GO:0008150",
                            "iri": "http://purl.obolibrary.org/obo/GO_0008150",
                            "label": "biological_process",
                            "short_form": "GO_0008150",
                            "ontology_name": "go",
                            "description": ["synthetic definition"],
                            "type": "class",
                            "is_defining_ontology": true
                        }]
                    }
                }))
                .into_response()
            } else if path == "/api/ontologies/go/terms" {
                axum::Json(json!({
                    "_embedded": {"terms": [{
                        "obo_id": "GO:0008150",
                        "iri": "http://purl.obolibrary.org/obo/GO_0008150",
                        "label": "biological_process",
                        "short_form": "GO_0008150",
                        "description": ["synthetic definition"],
                        "is_obsolete": false,
                        "has_children": true
                    }]}
                }))
                .into_response()
            } else if path.contains("/hierarchicalChildren") || path.contains("/parents") {
                axum::Json(json!({
                    "_embedded": {"terms": [{
                        "obo_id": "GO:0009987",
                        "iri": "http://purl.obolibrary.org/obo/GO_0009987",
                        "label": "cellular process",
                        "ontology_name": "go"
                    }]},
                    "page": {"size": 20, "totalElements": 1, "totalPages": 1, "number": 0}
                }))
                .into_response()
            } else {
                (StatusCode::NOT_FOUND, path.to_string()).into_response()
            }
        }
    };
    let app = Router::new()
        .route("/api/ontologies", get(capture.clone()))
        .route("/api/ontologies/{id}", get(capture.clone()))
        .route("/api/search", get(capture.clone()))
        .route("/api/ontologies/{id}/terms", get(capture.clone()))
        .route("/api/ontologies/{id}/terms/{iri}/{relation}", get(capture));
    let (bio, server) = serve(app).await;
    let catalogue = bio.call("list_ontologies", &json!({})).await.unwrap();
    let listed = bio
        .call(
            "list_ontologies",
            &json!({"ontology_ids": ["go", "missing"]}),
        )
        .await
        .unwrap();
    let search = bio
        .call(
            "search_ontology_terms",
            &json!({"query": "biological process", "ontologies": ["GO"], "exact": true, "max_results": 1}),
        )
        .await
        .unwrap();
    let term = bio
        .call(
            "get_ontology_term",
            &json!({"ontology": "go", "term_id": "GO:0008150"}),
        )
        .await
        .unwrap();
    let children = bio
        .call(
            "get_ontology_term",
            &json!({
                "ontology": "go",
                "term_id": "GO:0008150",
                "relation": "hierarchicalChildren"
            }),
        )
        .await
        .unwrap();
    server.abort();
    let urls = seen.lock().unwrap().join("\n");
    assert!(urls.contains("/api/search"), "{urls}");
    assert!(urls.contains("exact=true"), "{urls}");
    assert!(urls.contains("ontology=go"), "{urls}");
    assert_eq!(catalogue["source"], "OLS4");
    assert_eq!(catalogue["complete"], true);
    assert_eq!(
        catalogue["records"][0]["url"],
        "https://www.ebi.ac.uk/ols4/ontologies/go"
    );
    assert_eq!(listed["not_found"], json!(["missing"]));
    assert_eq!(search["total_found"], 8);
    assert_eq!(search["truncated"], true);
    assert!(search["terms"][0]["url"]
        .as_str()
        .unwrap()
        .contains("/ontologies/go/classes/"));
    assert_eq!(term["curie"], "GO:0008150");
    assert_eq!(term["parents"][0]["curie"], "GO:0009987");
    assert_eq!(children["relation"], "hierarchicalChildren");
    assert_eq!(children["returned"], 1);
}

#[tokio::test]
async fn reactome_maps_pathways_per_identifier() {
    let seen = Arc::new(StdMutex::new(Vec::<String>::new()));
    let traffic = seen.clone();
    let identifiers = {
        let traffic = traffic.clone();
        move |uri: Uri, body: String| {
            let traffic = traffic.clone();
            async move {
                traffic
                    .lock()
                    .unwrap()
                    .push(format!("identifiers {} {body}", uri.query().unwrap_or("")));
                axum::Json(json!({
                    "summary": {"token": "tok-synthetic"},
                    "identifiersNotFound": 1,
                    "pathwaysFound": 2,
                    "pathways": [{
                        "stId": "R-HSA-1640170",
                        "name": "Cell Cycle",
                        "llp": true,
                        "inDisease": false,
                        "species": {"name": "Homo sapiens", "taxId": "9606"},
                        "entities": {"found": 1, "total": 10, "fdr": 0.01, "pValue": 0.001},
                        "reactions": {"found": 2, "total": 20}
                    }]
                }))
            }
        }
    };
    let app = Router::new()
        .route("/AnalysisService/identifiers/", post(identifiers.clone()))
        .route("/AnalysisService/identifiers", post(identifiers))
        .route(
            "/AnalysisService/token/{token}/found/all",
            post({
                let traffic = traffic.clone();
                move |Path(token): Path<String>, body: String| {
                    let traffic = traffic.clone();
                    async move {
                        traffic
                            .lock()
                            .unwrap()
                            .push(format!("found {token} {body}"));
                        axum::Json(json!([{
                            "pathway": "R-HSA-1640170",
                            "entities": [{"id": "TP53"}]
                        }]))
                    }
                }
            }),
        )
        .route("/AnalysisService/database/version", get(|| async { "90" }));
    let (bio, server) = serve(app).await;
    let compact = bio
        .call(
            "map_reactome_pathways",
            &json!({"identifiers": ["TP53", "NOGENE"], "id_type": "symbol"}),
        )
        .await
        .unwrap();
    let full = bio
        .call(
            "map_reactome_pathways",
            &json!({
                "identifiers": ["TP53"],
                "compact": false,
                "resource": "UNIPROT",
                "include_disease": false
            }),
        )
        .await
        .unwrap();
    server.abort();
    let urls = seen.lock().unwrap().join("\n");
    assert!(urls.contains("TP53"), "{urls}");
    assert!(urls.contains("includeDisease=true"), "{urls}");
    assert!(urls.contains("resource=UNIPROT"), "{urls}");
    assert_eq!(compact["source"], "Reactome Analysis Service");
    assert_eq!(
        compact["source_url"],
        "https://reactome.org/AnalysisService"
    );
    assert_eq!(compact["reactome_version"], "90");
    assert_eq!(compact["missing_identifiers"], json!(["NOGENE"]));
    assert_eq!(compact["genes"]["TP53"]["found"], true);
    assert_eq!(
        compact["genes"]["TP53"]["pathways"][0]["url"],
        "https://reactome.org/content/detail/R-HSA-1640170"
    );
    assert_eq!(full["token"], "tok-synthetic");
    assert_eq!(full["pathways"][0]["entities_fdr"], 0.01);
    assert!(full["browser_url"]
        .as_str()
        .unwrap()
        .contains("ANALYSIS=tok-synthetic"));
}

#[tokio::test]
async fn rejects_rate_limits_malformed_json_html_and_oversized_bodies() {
    for (status, body, expected) in [
        (
            StatusCode::TOO_MANY_REQUESTS,
            "secret-token".into(),
            "HTTP 429",
        ),
        (StatusCode::OK, "{not-json".into(), "invalid JSON"),
        (
            StatusCode::OK,
            "<!doctype html><html><body>app</body></html>".into(),
            "HTML",
        ),
        (
            StatusCode::OK,
            " ".repeat(MAX_RESPONSE + 1),
            "exceeded 4 MiB",
        ),
    ] {
        let app = Router::new().route(
            "/v3/query",
            post({
                let body = body.clone();
                move || {
                    let body = body.clone();
                    async move { (status, [("retry-after", "60")], body).into_response() }
                }
            }),
        );
        let (bio, server) = serve(app).await;
        let error = bio
            .call("query_genes", &json!({"terms": ["TP53"]}))
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
async fn reactome_rate_limit_does_not_echo_request_body() {
    let limited = || async {
        (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "60")],
            "secret-token",
        )
            .into_response()
    };
    let app = Router::new()
        .route("/AnalysisService/identifiers/", post(limited))
        .route(
            "/AnalysisService/identifiers",
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
        .call("map_reactome_pathways", &json!({"identifiers": ["TP53"]}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("HTTP 429"), "{error}");
    assert!(!error.contains("secret-token"), "{error}");
}

#[tokio::test]
async fn kegg_operations_are_not_dispatched() {
    let (bio, server) = serve(Router::new()).await;
    for name in ["get_kegg_entries", "search_kegg", "link_kegg_ids"] {
        let error = genes_ontologies_call(&bio, name).await;
        assert!(error.contains("unknown native biological tool"), "{error}");
    }
    server.abort();
}

async fn genes_ontologies_call(bio: &NativeBio, name: &str) -> String {
    crate::genes_ontologies::call(bio, name, &json!({"ids": ["hsa:7157"]}))
        .await
        .unwrap_err()
        .to_string()
}
