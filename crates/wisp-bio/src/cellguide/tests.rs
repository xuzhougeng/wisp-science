use super::*;
use crate::http::{Http, MAX_RESPONSE};
use axum::{
    extract::Path,
    http::{StatusCode, Uri},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde_json::json;
use std::sync::{Arc, Mutex as StdMutex};

const SNAPSHOT: &str = "1700000001";

fn secretory() -> Value {
    json!({
        "id": "CL:0999001",
        "name": "wisp test secretory cell",
        "clDescription": "Invented secretory epithelial cell used only in tests.",
        "synonyms": ["test acinar analog", "synthetic acinus cell"]
    })
}

fn lymphocyte() -> Value {
    json!({
        "id": "CL:0999002",
        "name": "wisp test lymphocyte",
        "clDescription": "Invented lymphocyte used only in tests.",
        "synonyms": ["test T analog"]
    })
}

fn metadata() -> Value {
    json!({
        "CL:0999001": secretory(),
        "CL:0999002": lymphocyte()
    })
}

fn test_bio(base: &str) -> NativeBio {
    NativeBio::test_client(
        &[(
            "CELLGUIDE_BASE_URL".into(),
            base.trim_end_matches('/').into(),
        )],
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

fn catalog_routes() -> Router {
    Router::new()
        .route("/latest_snapshot_identifier", get(|| async { SNAPSHOT }))
        .route(
            "/{snapshot}/celltype_metadata.json",
            get(|Path(snapshot): Path<String>| async move {
                assert_eq!(snapshot, SNAPSHOT);
                Json(metadata())
            }),
        )
        .route(
            "/{snapshot}/tissue_metadata.json",
            get(|| async {
                Json(json!({
                    "UBERON:0999001": {
                        "id": "UBERON:0999001",
                        "name": "wisp test pancreas",
                        "uberonDescription": "Invented pancreas analog."
                    },
                    "UBERON:0999002": {
                        "id": "UBERON:0999002",
                        "name": "wisp test lung",
                        "uberonDescription": "Invented lung analog."
                    }
                }))
            }),
        )
        .route(
            "/{snapshot}/ontology_tree/NCBITaxon_9606/celltype_to_tissue_mapping.json",
            get(|| async {
                Json(json!({
                    "CL:0999001": ["UBERON:0999001", "UBERON:0999002", "UBERON:0999003"],
                    "CL:0999002": ["UBERON:0999002"]
                }))
            }),
        )
}

fn fixture_app() -> Router {
    catalog_routes()
        .route(
            "/validated_descriptions/{file}",
            get(|Path(file): Path<String>| async move {
                if file == "CL_0999001.json" {
                    Json(json!({
                        "description": "Curator-validated invented description of the secretory test cell.",
                        "references": ["https://doi.org/10.example/secretory", "https://doi.org/10.example/second"]
                    }))
                    .into_response()
                } else {
                    StatusCode::NOT_FOUND.into_response()
                }
            }),
        )
        .route(
            "/gpt_descriptions/{file}",
            get(|Path(file): Path<String>| async move {
                if file == "CL_0999002.json" {
                    Json("Invented GPT draft describing the lymphocyte test cell.").into_response()
                } else {
                    Json("should not be used when a validated description exists").into_response()
                }
            }),
        )
        .route(
            "/{snapshot}/computational_marker_genes/{file}",
            get(|Path((_snapshot, file)): Path<(String, String)>| async move {
                if file != "CL_0999001.json" {
                    return StatusCode::NOT_FOUND.into_response();
                }
                Json(json!([
                    {
                        "symbol": "LOW1",
                        "name": "low score gene",
                        "gene_ontology_term_id": "ENSG0999002",
                        "marker_score": 0.2,
                        "specificity": 0.1,
                        "me": 0.5,
                        "pc": 0.2,
                        "groupby_dims": {
                            "organism_ontology_term_label": "Homo sapiens",
                            "tissue_ontology_term_label": "wisp test lung"
                        }
                    },
                    {
                        "symbol": "HIGH1",
                        "name": "high score gene",
                        "gene_ontology_term_id": "ENSG0999001",
                        "marker_score": 1.8,
                        "specificity": 0.9,
                        "me": 3.2,
                        "pc": 0.85,
                        "groupby_dims": {
                            "organism_ontology_term_label": "Homo sapiens",
                            "tissue_ontology_term_label": "wisp test pancreas"
                        }
                    },
                    {
                        "symbol": "MID1",
                        "name": "mid score gene",
                        "marker_score": 1.1,
                        "specificity": 0.5,
                        "me": 1.0,
                        "pc": 0.4,
                        "groupby_dims": {"organism_ontology_term_label": "Homo sapiens"}
                    }
                ]))
                .into_response()
            }),
        )
        .route(
            "/{snapshot}/canonical_marker_genes/{file}",
            get(|Path((_snapshot, file)): Path<(String, String)>| async move {
                if file != "CL_0999001.json" {
                    return StatusCode::NOT_FOUND.into_response();
                }
                Json(json!([{
                    "tissue": "wisp test pancreas",
                    "symbol": "CANON1",
                    "name": "canonical test gene",
                    "publication": "10.example/one;;10.example/two",
                    "publication_titles": "Invented paper one;;Invented paper two"
                }]))
                .into_response()
            }),
        )
        .route(
            "/{snapshot}/source_collections/{file}",
            get(|Path((_snapshot, file)): Path<(String, String)>| async move {
                if file != "CL_0999001.json" {
                    return StatusCode::NOT_FOUND.into_response();
                }
                Json(json!([{
                    "collection_name": "Invented pancreas atlas",
                    "collection_url": "https://cellxgene.cziscience.com/collections/00000000-0000-0000-0000-000000000001",
                    "publication_title": "Invented atlas preprint",
                    "publication_url": "https://doi.org/10.example/atlas",
                    "tissue": [{"label": "wisp test pancreas", "ontology_term_id": "UBERON:0999001"}],
                    "disease": [{"label": "normal", "ontology_term_id": "PATO:0000461"}],
                    "organism": [{"label": "Homo sapiens", "ontology_term_id": "NCBITaxon:9606"}]
                }]))
                .into_response()
            }),
        )
}

#[test]
fn catalog_registers_five_read_only_cellguide_tools() {
    let names: Vec<_> = catalog()
        .into_iter()
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        names,
        vec![
            ("cellguide", "search_cell_types".into()),
            ("cellguide", "get_cell_type_info".into()),
            ("cellguide", "get_marker_genes".into()),
            ("cellguide", "get_source_data".into()),
            ("cellguide", "get_cell_tissues".into()),
        ]
    );
    assert!(crate::contains_tool("get_cell_type_info"));
    assert_eq!(
        crate::domain_for_tool("get_marker_genes"),
        Some("cellguide")
    );
    assert!(crate::package_selects("mcp_cellguide", "cellguide"));
    assert!(crate::selected_by_package("mcp_cellguide"));
}

#[test]
fn rejects_unbounded_or_malformed_arguments() {
    for args in [
        json!({"query": " "}),
        json!({"query": "x", "limit": 0}),
        json!({"query": "x", "limit": 51}),
        json!({"query": "x", "api_key": "secret"}),
        json!({}),
    ] {
        assert!(
            serde_json::from_value::<SearchArgs>(args.clone())
                .ok()
                .is_none_or(|search| require_text(&search.query, "query").is_err()
                    || bound_limit(search.limit, MAX_SEARCH, "limit").is_err()),
            "{args}"
        );
    }
    for args in [
        json!({"cell_type": ""}),
        json!({"cell_type": "ok", "api_key": "secret"}),
        json!({"cell_type": "CL:0999001", "marker_type": "computational", "limit": 101}),
        json!({"cell_type": "a/b"}),
    ] {
        let markers = serde_json::from_value::<MarkerArgs>(args.clone());
        let cell = serde_json::from_value::<CellTypeArgs>(args.clone());
        match (markers, cell) {
            (Ok(parsed), _) => assert!(
                require_text(&parsed.cell_type, "cell_type").is_err()
                    || bound_limit(parsed.limit, MAX_MARKERS, "limit").is_err()
                    || !matches!(parsed.marker_type.as_str(), "computational" | "canonical"),
                "{args}"
            ),
            (_, Ok(parsed)) => assert!(
                require_text(&parsed.cell_type, "cell_type").is_err(),
                "{args}"
            ),
            (Err(_), Err(_)) => {}
        }
    }
}

#[test]
fn normalizes_cell_ontology_and_uberon_identifiers() {
    assert_eq!(parse_cl_id("CL:0999001").as_deref(), Some("CL:0999001"));
    assert_eq!(parse_cl_id("cl_0999001").as_deref(), Some("CL:0999001"));
    assert_eq!(parse_cl_id("0999001").as_deref(), Some("CL:0999001"));
    assert_eq!(parse_cl_id("622"), None);
    assert_eq!(parse_cl_id("CL:622"), None);
    assert_eq!(parse_cl_id("not-an-id"), None);
    assert_eq!(
        normalize_uberon("UBERON_0999001").as_deref(),
        Some("UBERON:0999001")
    );
    assert_eq!(filesystem_id("CL:0999001"), "CL_0999001");
}

#[test]
fn ranks_names_ahead_of_synonym_substrings() {
    let catalog = parse_cell_catalog(&metadata()).unwrap();
    let ranked: Vec<_> = rank_cells(&catalog, "wisp test lymphocyte")
        .into_iter()
        .map(|cell| cell.id)
        .collect();
    assert_eq!(ranked[0], "CL:0999002");
    let analog: Vec<_> = rank_cells(&catalog, "acinar analog")
        .into_iter()
        .map(|cell| cell.name)
        .collect();
    assert_eq!(analog, vec!["wisp test secretory cell"]);
    assert!(rank_cells(&catalog, "missing cell").is_empty());
    assert_eq!(
        resolve_cell(&catalog, "CL_0999001", SNAPSHOT).unwrap().name,
        "wisp test secretory cell"
    );
}

#[tokio::test]
async fn search_dispatches_through_native_bio_and_reports_source_urls() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = captured.clone();
    let app = Router::new()
        .route(
            "/latest_snapshot_identifier",
            get({
                let seen = seen.clone();
                move |uri: Uri| {
                    seen.lock().unwrap().push(uri.path().to_string());
                    async { SNAPSHOT }
                }
            }),
        )
        .route(
            "/{snapshot}/celltype_metadata.json",
            get({
                let seen = seen.clone();
                move |uri: Uri| {
                    seen.lock().unwrap().push(uri.path().to_string());
                    async { Json(metadata()) }
                }
            }),
        );
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "search_cell_types",
            &json!({"query": "lymphocyte", "limit": 1}),
        )
        .await
        .unwrap();
    let extra = bio
        .call("search_cell_types", &json!({"query": "wisp test"}))
        .await
        .unwrap();
    server.abort();
    let paths = captured.lock().unwrap().join(" ");
    assert!(paths.contains("/latest_snapshot_identifier"), "{paths}");
    assert!(
        paths.contains(&format!("/{SNAPSHOT}/celltype_metadata.json")),
        "{paths}"
    );
    assert_eq!(result["source"], "CELLxGENE CellGuide");
    assert_eq!(result["source_url"], CELLGUIDE_UI);
    assert_eq!(result["snapshot"], SNAPSHOT);
    assert_eq!(result["total_available"], 1);
    assert_eq!(result["returned"], 1);
    assert_eq!(result["truncated"], false);
    assert_eq!(result["records"][0]["id"], "CL:0999002");
    assert_eq!(
        result["records"][0]["url"],
        "https://cellxgene.cziscience.com/cellguide/CL_0999002"
    );
    assert_eq!(extra["total_available"], 2);
    assert_eq!(extra["returned"], 2);
    assert_eq!(extra["truncated"], false);
}

#[tokio::test]
async fn cell_type_info_prefers_validated_description_and_card_url() {
    let (bio, server) = serve(fixture_app()).await;
    let result = bio
        .call("get_cell_type_info", &json!({"cell_type": "CL_0999001"}))
        .await
        .unwrap();
    let gpt = bio
        .call(
            "get_cell_type_info",
            &json!({"cell_type": "wisp test lymphocyte"}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["id"], "CL:0999001");
    assert_eq!(result["name"], "wisp test secretory cell");
    assert_eq!(result["description_source"], "validated");
    assert_eq!(
        result["description"],
        "Curator-validated invented description of the secretory test cell."
    );
    assert_eq!(
        result["references"],
        json!([
            "https://doi.org/10.example/secretory",
            "https://doi.org/10.example/second"
        ])
    );
    assert_eq!(
        result["url"],
        "https://cellxgene.cziscience.com/cellguide/CL_0999001"
    );
    assert_eq!(result["source_url"], CELLGUIDE_UI);
    assert_eq!(gpt["id"], "CL:0999002");
    assert_eq!(gpt["description_source"], "gpt");
    assert_eq!(
        gpt["description"],
        "Invented GPT draft describing the lymphocyte test cell."
    );
}

#[tokio::test]
async fn remaining_tools_dispatch_through_native_bio_call() {
    let (bio, server) = serve(fixture_app()).await;
    let markers = bio
        .call(
            "get_marker_genes",
            &json!({"cell_type": "test acinar analog", "limit": 2}),
        )
        .await
        .unwrap();
    let canonical = bio
        .call(
            "get_marker_genes",
            &json!({"cell_type": "CL:0999001", "marker_type": "canonical"}),
        )
        .await
        .unwrap();
    let sources = bio
        .call("get_source_data", &json!({"cell_type": "0999001"}))
        .await
        .unwrap();
    let tissues = bio
        .call("get_cell_tissues", &json!({"cell_type": "CL:0999001"}))
        .await
        .unwrap();
    let empty_markers = bio
        .call("get_marker_genes", &json!({"cell_type": "CL:0999002"}))
        .await
        .unwrap();
    server.abort();
    assert_eq!(markers["marker_type"], "computational");
    assert_eq!(markers["total_available"], 3);
    assert_eq!(markers["returned"], 2);
    assert_eq!(markers["truncated"], true);
    assert_eq!(markers["marker_genes"][0]["symbol"], "HIGH1");
    assert_eq!(markers["marker_genes"][0]["mean_expression"], 3.2);
    assert_eq!(markers["marker_genes"][0]["fraction_expressed"], 0.85);
    assert_eq!(markers["marker_genes"][0]["tissue"], "wisp test pancreas");
    assert_eq!(
        markers["artifact_url"],
        format!("{CELLGUIDE_CDN}/{SNAPSHOT}/computational_marker_genes/CL_0999001.json")
    );
    assert_eq!(canonical["marker_genes"][0]["symbol"], "CANON1");
    assert_eq!(
        canonical["marker_genes"][0]["publication"],
        json!(["10.example/one", "10.example/two"])
    );
    assert_eq!(
        sources["collections"][0]["collection_name"],
        "Invented pancreas atlas"
    );
    assert_eq!(
        sources["collections"][0]["tissues"][0]["id"],
        "UBERON:0999001"
    );
    assert_eq!(tissues["organism"], "NCBITaxon:9606");
    assert_eq!(tissues["total_available"], 3);
    assert_eq!(tissues["returned"], 3);
    assert_eq!(tissues["tissues"][0]["name"], "wisp test pancreas");
    assert_eq!(tissues["tissues"][2]["id"], "UBERON:0999003");
    assert_eq!(tissues["tissues"][2]["name"], "UBERON:0999003");
    assert_eq!(
        tissues["tissues"][0]["url"],
        "https://cellxgene.cziscience.com/cellguide/UBERON_0999001"
    );
    assert_eq!(empty_markers["total_available"], 0);
    assert_eq!(empty_markers["marker_genes"], json!([]));
}

#[tokio::test]
async fn unknown_cell_type_is_an_error_not_success_shaped_evidence() {
    let (bio, server) = serve(fixture_app()).await;
    let error = bio
        .call("get_cell_type_info", &json!({"cell_type": "missing cell"}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("was not found"), "{error}");
    assert!(!error.contains("\"error\""));
}

#[tokio::test]
async fn rejects_rate_limits_malformed_json_html_and_does_not_echo_bodies() {
    for (status, body, expected) in [
        (
            StatusCode::TOO_MANY_REQUESTS,
            "secret-token".to_string(),
            "HTTP 429",
        ),
        (StatusCode::OK, "{not-json".to_string(), "invalid JSON"),
        (
            StatusCode::OK,
            "<!doctype html><html><body>app</body></html>".to_string(),
            "HTML page",
        ),
        (StatusCode::OK, " ".to_string(), "empty JSON body"),
        (
            StatusCode::OK,
            "../not-a-snapshot".to_string(),
            "snapshot identifier",
        ),
    ] {
        let app = Router::new()
            .route(
                "/latest_snapshot_identifier",
                get({
                    let body = body.clone();
                    move || {
                        let body = body.clone();
                        async move { (status, [("retry-after", "60")], body).into_response() }
                    }
                }),
            )
            .route(
                "/{snapshot}/celltype_metadata.json",
                get(|| async { Json(metadata()) }),
            );
        let snapshot_cases =
            expected == "HTTP 429" || expected == "HTML page" || expected == "snapshot identifier";
        let (bio, server) = if snapshot_cases {
            serve(app).await
        } else {
            let app = Router::new()
                .route("/latest_snapshot_identifier", get(|| async { SNAPSHOT }))
                .route(
                    "/{snapshot}/celltype_metadata.json",
                    get({
                        let body = body.clone();
                        move || {
                            let body = body.clone();
                            async move { (StatusCode::OK, body).into_response() }
                        }
                    }),
                );
            serve(app).await
        };
        let error = bio
            .call("search_cell_types", &json!({"query": "lymphocyte"}))
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
async fn oversized_catalog_is_rejected() {
    let app = Router::new()
        .route("/latest_snapshot_identifier", get(|| async { SNAPSHOT }))
        .route(
            "/{snapshot}/celltype_metadata.json",
            get(|| async { (StatusCode::OK, " ".repeat(MAX_RESPONSE + 1)).into_response() }),
        );
    let (bio, server) = serve(app).await;
    let error = bio
        .call("search_cell_types", &json!({"query": "lymphocyte"}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("exceeded 4 MiB"), "{error}");
}

#[tokio::test]
async fn unknown_tool_name_is_rejected() {
    let (bio, server) = serve(Router::new()).await;
    let error = call(&bio, "cellguide_not_a_tool", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("unknown native biological tool"));
}

#[tokio::test]
async fn search_truncates_to_requested_limit() {
    let (bio, server) = serve(fixture_app()).await;
    let result = bio
        .call(
            "search_cell_types",
            &json!({"query": "wisp test", "limit": 1}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["total_available"], 2);
    assert_eq!(result["returned"], 1);
    assert_eq!(result["truncated"], true);
}
