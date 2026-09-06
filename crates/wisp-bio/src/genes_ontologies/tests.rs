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
            ("REACTOME_BASE_URL".into(), base.clone()),
            ("KEGG_BASE_URL".into(), base),
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
fn catalog_registers_ten_genes_ontologies_tools_including_kegg() {
    let names: Vec<_> = catalog()
        .into_iter()
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        names,
        vec![
            ("genes-ontologies", "get_go_annotations".into()),
            ("genes-ontologies", "get_kegg_entries".into()),
            ("genes-ontologies", "get_ontology_term".into()),
            ("genes-ontologies", "get_uniprot_entries".into()),
            ("genes-ontologies", "link_kegg_ids".into()),
            ("genes-ontologies", "list_ontologies".into()),
            ("genes-ontologies", "map_reactome_pathways".into()),
            ("genes-ontologies", "query_genes".into()),
            ("genes-ontologies", "search_kegg".into()),
            ("genes-ontologies", "search_ontology_terms".into()),
        ]
    );
    for kegg in ["get_kegg_entries", "search_kegg", "link_kegg_ids"] {
        assert!(crate::contains_tool(kegg), "{kegg}");
        assert_eq!(crate::domain_for_tool(kegg), Some("genes-ontologies"));
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
async fn kegg_rejects_malformed_arguments() {
    let (bio, server) = serve(Router::new()).await;
    let empty = bio
        .call("get_kegg_entries", &json!({"ids": []}))
        .await
        .unwrap_err()
        .to_string();
    assert!(empty.contains("at least one"), "{empty}");
    let duplicates = bio
        .call(
            "get_kegg_entries",
            &json!({"ids": ["syn:2001", "syn:2001"]}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(duplicates.contains("duplicate"), "{duplicates}");
    let too_many: Vec<String> = (1..=51).map(|n| format!("syn:{n}")).collect();
    let overflow = bio
        .call("get_kegg_entries", &json!({"ids": too_many}))
        .await
        .unwrap_err()
        .to_string();
    assert!(overflow.contains("50"), "{overflow}");
    let unknown_op = bio
        .call(
            "link_kegg_ids",
            &json!({"ids": ["syn:2001"], "target_db": "pathway", "operation": "dump"}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        unknown_op.contains("operation") && unknown_op.contains("link"),
        "{unknown_op}"
    );
    let formula_on_org = bio
        .call(
            "search_kegg",
            &json!({"query": "C6H12O6", "database": "hsa", "option": "formula"}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        formula_on_org.contains("compound") || formula_on_org.contains("drug"),
        "{formula_on_org}"
    );
    let extra = bio
        .call(
            "get_kegg_entries",
            &json!({"ids": ["syn:2001"], "api_key": "secret-token"}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        extra.contains("unknown field") || extra.contains("invalid get_kegg_entries"),
        "{extra}"
    );
    assert!(!extra.contains("secret-token"), "{extra}");
    server.abort();
}

#[tokio::test]
async fn get_kegg_entries_batches_and_parses_flat_file() {
    let seen = Arc::new(StdMutex::new(Vec::<String>::new()));
    let traffic = seen.clone();
    let app = Router::new().route(
        "/get/{*ids}",
        get(move |uri: Uri| {
            let traffic = traffic.clone();
            async move {
                traffic.lock().unwrap().push(uri.to_string());
                let rest = uri.path().strip_prefix("/get/").unwrap_or(uri.path());
                let decoded = decode_kegg_path(rest);
                let mut body = String::new();
                for id in decoded.split('+') {
                    let local = id.rsplit_once(':').map(|(_, rest)| rest).unwrap_or(id);
                    let Ok(n) = local.parse::<u32>() else {
                        continue;
                    };
                    if !(2001..=2011).contains(&n) {
                        continue;
                    }
                    if !body.is_empty() {
                        body.push('\n');
                    }
                    body.push_str(&synthetic_gene_entry(id));
                }
                body
            }
        }),
    );
    let (bio, server) = serve(app).await;
    let ids: Vec<String> = (2001..=2011).map(|n| format!("syn:{n}")).collect();
    let result = bio
        .call(
            "get_kegg_entries",
            &json!({"ids": ids, "include_raw": true}),
        )
        .await
        .unwrap();
    let missing = bio
        .call(
            "get_kegg_entries",
            &json!({"ids": ["syn:2001", "syn:2099"]}),
        )
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    let urls = seen.lock().unwrap().clone();
    assert_eq!(urls.len(), 3, "{urls:?}");
    assert!(
        urls[0].contains("syn:2001")
            && urls[0].contains("syn:2010")
            && !urls[0].contains("syn:2011"),
        "{}",
        urls[0]
    );
    assert!(
        urls[1].contains("syn:2011") && !urls[1].contains("syn:2001"),
        "{}",
        urls[1]
    );
    assert_eq!(plus_count(&urls[0]), 9);
    assert_eq!(result["source"], "KEGG");
    assert_eq!(
        result["source_url"],
        "https://rest.kegg.jp/get/syn:2001+syn:2002+syn:2003+syn:2004+syn:2005+syn:2006+syn:2007+syn:2008+syn:2009+syn:2010"
    );
    assert_eq!(result["returned"], 11);
    assert_eq!(result["records"][0]["requested_id"], "syn:2001");
    assert_eq!(result["records"][0]["entry_id"], "2001");
    assert_eq!(result["records"][0]["entry_type"], "CDS");
    assert_eq!(result["records"][0]["name"], json!(["SYN2001"]));
    assert_eq!(
        result["records"][0]["symbol"],
        json!(["SYN2001", "ALT2001"])
    );
    assert_eq!(
        result["records"][0]["pathway"],
        json!([
            {"id": "syn00010", "name": "Synthetic glycolysis"},
            {"id": "syn00020", "name": "Synthetic citrate cycle"}
        ])
    );
    assert_eq!(
        result["records"][0]["url"],
        "https://www.kegg.jp/entry/syn:2001"
    );
    assert!(result["records"][0]["raw"]
        .as_str()
        .unwrap()
        .contains("///"));
    assert_eq!(result["records"][10]["requested_id"], "syn:2011");
    assert!(missing.contains("syn:2099"), "{missing}");
}

#[tokio::test]
async fn search_kegg_parses_find_tsv_and_filters_exact_symbols() {
    let app = Router::new().route(
        "/find/{*rest}",
        get(|uri: Uri| async move {
            let rest = uri.path().strip_prefix("/find/").unwrap_or(uri.path());
            if rest.contains("C6H12O6") {
                return "syn:C99999\tsynthetic hexose\n".to_string();
            }
            "\
syn:2001\tSYN1, ALT1; synthetic protein one\n\
syn:2101\tSYN1BP, OTHER; synthetic binding protein\n\
syn:2003\tSYN1; synthetic protein one isoform\n\
syn:3000\tUNRELATED; decoy record\n"
                .to_string()
        }),
    );
    let (bio, server) = serve(app).await;
    let page = bio
        .call(
            "search_kegg",
            &json!({"query": "SYN1", "database": "syn", "max_hits": 2}),
        )
        .await
        .unwrap();
    let exact = bio
        .call(
            "search_kegg",
            &json!({
                "query": "SYN1",
                "database": "syn",
                "exact_gene_symbol": true
            }),
        )
        .await
        .unwrap();
    let none = bio
        .call(
            "search_kegg",
            &json!({
                "query": "NOSUCH",
                "database": "syn",
                "exact_gene_symbol": true
            }),
        )
        .await
        .unwrap();
    let formula = bio
        .call(
            "search_kegg",
            &json!({
                "query": "C6H12O6",
                "database": "compound",
                "option": "formula"
            }),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(page["source"], "KEGG");
    assert_eq!(page["source_url"], "https://rest.kegg.jp/find/syn/SYN1");
    assert_eq!(page["total_hits"], 4);
    assert_eq!(page["returned"], 2);
    assert_eq!(page["truncated"], true);
    assert_eq!(page["records"][0]["entry_id"], "syn:2001");
    assert_eq!(exact["n_matches"], 2);
    assert_eq!(exact["total_hits"], 2);
    assert_eq!(exact["truncated"], false);
    let exact_ids: Vec<_> = exact["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["entry_id"].as_str().unwrap())
        .collect();
    assert_eq!(exact_ids, vec!["syn:2001", "syn:2003"]);
    assert_eq!(none["n_matches"], 0);
    assert_eq!(none["returned"], 0);
    assert_eq!(formula["records"][0]["entry_id"], "syn:C99999");
    assert_eq!(
        formula["source_url"],
        "https://rest.kegg.jp/find/compound/C6H12O6/formula"
    );
}

#[tokio::test]
async fn link_kegg_ids_maps_tsv_reports_missing_and_batches() {
    let seen = Arc::new(StdMutex::new(Vec::<String>::new()));
    let traffic = seen.clone();
    let app = Router::new()
        .route(
            "/link/{*rest}",
            get(move |uri: Uri| {
                let traffic = traffic.clone();
                async move {
                    traffic.lock().unwrap().push(uri.to_string());
                    let rest = uri.path().strip_prefix("/link/").unwrap_or(uri.path());
                    let decoded = decode_kegg_path(rest);
                    let ids = decoded.split('/').nth(1).unwrap_or("");
                    let mut body = String::new();
                    for id in ids.split('+') {
                        let local = id.rsplit_once(':').map(|(_, rest)| rest).unwrap_or(id);
                        let echoed = format!("syn:{local}");
                        if local == "2001" {
                            body.push_str(&format!(
                                "{echoed}\tpath:syn00010\n{echoed}\tpath:syn00020\n"
                            ));
                        } else if local == "2002" {
                            body.push_str(&format!("{echoed}\tpath:syn00010\n"));
                        }
                    }
                    body
                }
            }),
        )
        .route(
            "/conv/{*rest}",
            get(|| async { "syn:2001\tncbi-geneid:9001\n".to_string() }),
        );
    let (bio, server) = serve(app).await;
    let mut ids: Vec<String> = vec!["2001".into()];
    ids.extend((2002..=2011).map(|n| format!("syn:{n}")));
    let result = bio
        .call(
            "link_kegg_ids",
            &json!({"ids": ids, "target_db": "pathway"}),
        )
        .await
        .unwrap();
    let conv = bio
        .call(
            "link_kegg_ids",
            &json!({
                "ids": ["syn:2001"],
                "target_db": "ncbi-geneid",
                "operation": "conv"
            }),
        )
        .await
        .unwrap();
    server.abort();
    let urls = seen.lock().unwrap().clone();
    assert_eq!(urls.len(), 2, "{urls:?}");
    assert!(urls[0].contains("/link/pathway/"), "{}", urls[0]);
    assert!(
        urls[0].contains("2001") && !urls[0].contains("syn:2011"),
        "{}",
        urls[0]
    );
    assert!(
        urls[1].contains("syn:2011") && !urls[1].contains("2001"),
        "{}",
        urls[1]
    );
    assert_eq!(plus_count(&urls[0]), 9);
    assert_eq!(result["source"], "KEGG");
    assert_eq!(result["operation"], "link");
    assert_eq!(result["target_db"], "pathway");
    assert_eq!(result["returned"], 3);
    assert_eq!(result["records"][0]["source_id"], "2001");
    assert_eq!(result["records"][0]["target_id"], "path:syn00010");
    let missing: Vec<_> = result["missing_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(missing.contains(&"syn:2011"), "{missing:?}");
    assert!(!missing.iter().any(|id| *id == "2001" || *id == "syn:2002"));
    assert_eq!(conv["operation"], "conv");
    assert_eq!(conv["records"][0]["target_id"], "ncbi-geneid:9001");
    assert_eq!(conv["missing_ids"], json!([]));
}

#[tokio::test]
async fn kegg_http_errors_do_not_echo_response_bodies() {
    for (status, expected) in [
        (StatusCode::BAD_REQUEST, "HTTP 400"),
        (StatusCode::TOO_MANY_REQUESTS, "HTTP 429"),
    ] {
        let app = Router::new().route(
            "/get/{*ids}",
            get(move || async move {
                (status, [("retry-after", "60")], "secret-token").into_response()
            }),
        );
        let (bio, server) = serve(app).await;
        let error = bio
            .call("get_kegg_entries", &json!({"ids": ["syn:2001"]}))
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
async fn kegg_oversized_body_uses_max_response() {
    let app = Router::new().route(
        "/get/{*ids}",
        get(|| async { (StatusCode::OK, " ".repeat(MAX_RESPONSE + 1)).into_response() }),
    );
    let (bio, server) = serve(app).await;
    let error = bio
        .call("get_kegg_entries", &json!({"ids": ["syn:2001"]}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("exceeded 4 MiB"), "{error}");
}

fn decode_kegg_path(value: &str) -> String {
    value
        .replace("%3A", ":")
        .replace("%3a", ":")
        .replace("%2B", "+")
        .replace("%2b", "+")
}

fn plus_count(url: &str) -> usize {
    url.matches('+').count() + url.matches("%2B").count() + url.matches("%2b").count()
}

fn synthetic_gene_entry(requested: &str) -> String {
    let local = requested
        .rsplit_once(':')
        .map(|(_, rest)| rest)
        .unwrap_or(requested);
    [
        format!("ENTRY       {local}              CDS       T00001"),
        format!("NAME        (RefSeq) SYN{local}"),
        format!("SYMBOL      SYN{local}, ALT{local}"),
        format!("DEFINITION  synthetic protein {local}"),
        "ORGANISM    syn  Synthetic organism".to_string(),
        "PATHWAY     syn00010  Synthetic glycolysis".to_string(),
        "            syn00020  Synthetic citrate cycle".to_string(),
        format!("ORTHOLOGY   K{local}  synthetic dehydrogenase {local}"),
        "///".to_string(),
        String::new(),
    ]
    .join("\n")
}
