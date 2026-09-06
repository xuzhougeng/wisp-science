use super::*;

#[test]
fn supplier_matches_include_codes_in_catalog_rows() {
    let records =
        vec![json!({"zinc_id":"ZINC000000000001","catalogs":[{"supplier_code":"m_test"}]})];
    assert_eq!(
        missing_supplier_codes(&["m_test".into(), "M_test".into()], &records),
        vec!["M_test"]
    );
}
use crate::http::{Http, MAX_RESPONSE};
use axum::{
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde_json::json;
use std::sync::{Arc, Mutex as StdMutex};

fn record() -> Value {
    json!({
        "zinc_id": "ZINC000000000099",
        "smiles": "CCO",
        "catalogs": [{"catalog_name": "sH15P090", "short_name": "s"}],
        "tranche_name": "H15P090"
    })
}

fn test_bio(base: &str) -> NativeBio {
    NativeBio::test_client(
        &[
            ("ZINC_BASE_URL".into(), base.trim_end_matches('/').into()),
            (
                "ZINC_SMALLWORLD_URL".into(),
                base.trim_end_matches('/').into(),
            ),
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

fn poll_success(result: Value) -> Router {
    Router::new().route(
        "/search/result/{task}",
        get(move |Path(_task): Path<String>| {
            let result = result.clone();
            async move { axum::Json(json!({"status": "SUCCESS", "result": result})) }
        }),
    )
}

#[test]
fn catalog_registers_five_read_only_zinc_tools() {
    let names: Vec<_> = catalog()
        .into_iter()
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        names,
        vec![
            ("zinc", "zinc_search_by_id".into()),
            ("zinc", "zinc_search_by_smiles".into()),
            ("zinc", "zinc_search_by_supplier".into()),
            ("zinc", "zinc_get_3d".into()),
            ("zinc", "zinc_random_sample".into()),
        ]
    );
    assert!(crate::contains_tool("zinc_search_by_id"));
    assert_eq!(crate::domain_for_tool("zinc_search_by_id"), Some("zinc"));
    assert!(crate::package_selects("mcp_zinc", "zinc"));
    assert!(crate::selected_by_package("mcp_zinc"));
}

#[test]
fn rejects_unbounded_or_malformed_identifiers() {
    for args in [
        json!({"zinc_ids": []}),
        json!({"zinc_ids": [" "]}),
        json!({"zinc_ids": ["ZINC000000000099,ZINC000000000100"]}),
        json!({"zinc_ids": ["not-zinc"]}),
        json!({"zinc_ids": ["ZINC000000000099"], "max_results": 0}),
        json!({"zinc_ids": ["ZINC000000000099"], "max_results": 501}),
        json!({"zinc_ids": ["ZINC000000000099"], "api_key": "secret"}),
        json!({"zinc_ids": vec!["ZINC000000000001"; 101]}),
    ] {
        let parsed = serde_json::from_value::<SearchById>(args.clone());
        match parsed {
            Ok(search) => assert!(
                require_ids(&search.zinc_ids, MAX_IDS, "ZINC id", true).is_err()
                    || bound_page(search.max_results).is_err(),
                "{args}"
            ),
            Err(_) => {}
        }
    }
    let ids = require_ids(
        &[" ZINC12 ".into(), "ZINCaa0000000Aaa".into()],
        MAX_IDS,
        "ZINC id",
        true,
    )
    .unwrap();
    assert_eq!(canonical_zinc_id(&ids[0]), "ZINC000000000012");
    assert_eq!(canonical_zinc_id(&ids[1]), "ZINCaa0000000Aaa");
}

#[test]
fn smiles_and_supplier_arguments_are_bounded() {
    for args in [
        json!({"smiles": " "}),
        json!({"smiles": "CCO", "dist": 11}),
        json!({"smiles": "CCO", "adist": -1}),
        json!({"smiles": "CCO", "api_key": "x"}),
    ] {
        if let Ok(search) = serde_json::from_value::<SearchBySmiles>(args.clone()) {
            let smiles = search.smiles.trim();
            assert!(
                smiles.is_empty()
                    || !(0..=10).contains(&search.dist)
                    || !search.adist.is_none_or(|d| (0..=10).contains(&d)),
                "{args}"
            );
        }
    }
    assert!(require_ids(&["SYNTH 1".into()], MAX_IDS, "supplier code", false).is_err());
    assert!(bound_page(0).is_err());
    assert!(parse_tranche("H15P090") == Some((15, 0.90, "H15P090".into())));
    assert_eq!(parse_tranche("H22M050").unwrap().1, -0.50);
    assert!(parse_tranche("nope").is_none());
    assert!(parse_tranche("../H15P090").is_none());
}

#[test]
fn flatten_preserves_sources_missing_ids_and_tranche_shapes() {
    let (records, counts) = flatten_result(&json!({
        "zinc20": [{"zinc_id": "ZINC12", "smiles": "C"}],
        "zinc22": [{
            "zinc_id": "ZINCaa0000000Aaa",
            "smiles": "CCO",
            "tranche": {"h_num": "H15", "p_num": "P090"},
            "catalogs": [{"catalog_name": "sH15P090"}]
        }],
        "missing": ["ignored"]
    }))
    .unwrap();
    assert_eq!(counts["zinc22"], 1);
    assert_eq!(counts["zinc20"], 1);
    assert_eq!(records[0]["source"], "zinc22");
    assert_eq!(records[0]["tranche_name"], "H15P090");
    assert_eq!(records[0]["tranche_properties"]["heavy_atoms"], 15);
    assert_eq!(
        records[0]["url"],
        "https://cartblanche22.docking.org/substance/ZINCaa0000000Aaa"
    );
    assert_eq!(records[1]["zinc_id"], "ZINC12");
    let missing = missing_zinc_ids(&["ZINC12".into(), "ZINC000000000077".into()], &records);
    assert_eq!(missing, vec!["ZINC000000000077"]);
    assert!(flatten_result(&json!("nope")).is_err());
}

#[tokio::test]
async fn search_by_id_dispatches_form_post_and_reports_missing_and_source_urls() {
    let captured = Arc::new(StdMutex::new(String::new()));
    let body = captured.clone();
    let app = Router::new()
        .route(
            "/substances.txt",
            post(move |incoming: String| {
                *body.lock().unwrap() = incoming;
                async { axum::Json(json!({"task": "11111111-1111-1111-1111-111111111111"})) }
            }),
        )
        .merge(poll_success(json!({
            "zinc22": [record()],
            "zinc20": []
        })));
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "zinc_search_by_id",
            &json!({
                "zinc_ids": ["ZINC99", "ZINC000000000077"],
                "max_results": 1
            }),
        )
        .await
        .unwrap();
    server.abort();
    let form = captured.lock().unwrap().clone();
    assert!(form.contains("zinc_ids=ZINC99"), "{form}");
    assert!(form.contains("output_fields=zinc_id"), "{form}");
    assert_eq!(result["source"], "ZINC CartBlanche22");
    assert_eq!(result["source_url"], CARTBLANCHE);
    assert_eq!(result["total_available"], 1);
    assert_eq!(result["returned"], 1);
    assert_eq!(result["truncated"], false);
    assert_eq!(result["missing_ids"], json!(["ZINC000000000077"]));
    assert_eq!(
        result["records"][0]["url"],
        "https://cartblanche22.docking.org/substance/ZINC000000000099"
    );
    assert_eq!(result["records"][0]["tranche_properties"]["logp"], 0.9);
    assert_eq!(
        result["query"]["zinc_ids"],
        json!(["ZINC99", "ZINC000000000077"])
    );
}

#[tokio::test]
async fn remaining_tools_dispatch_through_native_bio_call() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = captured.clone();
    let captured_for_smiles = captured.clone();
    let capture = move |path: &'static str| {
        let seen = seen.clone();
        move |incoming: String| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(format!("{path} {incoming}"));
                axum::Json(json!({"task": "task-zinc"}))
            }
        }
    };
    let app = Router::new()
        .route("/search/maps", get(|| async { axum::Json(json!({"zinc20-forsale-test":{"enabled":true}})) }))
        .route("/search/view", get(move |uri: axum::http::Uri| {
            let seen = captured_for_smiles.clone();
            async move {
                seen.lock().unwrap().push(format!("smiles {}", uri.query().unwrap_or("")));
                axum::Json(json!({"status":{"state":"DONE"},"recordsFiltered":1,"data":[[{"id":"99","hitSmiles":"CCO 99"},2,1]]}))
            }
        }))
        .route("/catitems.txt", post(capture("catitems")))
        .route("/substance/random.json", post(capture("random")))
        .route("/substances.txt", post(capture("substances")))
        .route("/substance/random/task-zinc.json", get(|| async {
            axum::Json(json!({"status":"SUCCESS","result": "[{\"zincid\":\"ZINC000000000099\",\"SMILES\":\"CCO\",\"tranche\":\"H15P090\"}]"}))
        }))
        .merge(poll_success(json!({
            "zinc22": [{
                "zinc_id": "ZINC000000000099",
                "smiles": "CCO",
                "supplier_code": "SYNTH-0001",
                "tranche_name": "H15P090",
                "catalogs": []
            }]
        })));
    let (bio, server) = serve(app).await;
    let smiles = bio
        .call(
            "zinc_search_by_smiles",
            &json!({"smiles": "CCO", "dist": 2, "adist": 1, "max_results": 10}),
        )
        .await
        .unwrap();
    let supplier = bio
        .call(
            "zinc_search_by_supplier",
            &json!({"supplier_codes": ["SYNTH-0001", "MISSING-1"]}),
        )
        .await
        .unwrap();
    let random = bio
        .call(
            "zinc_random_sample",
            &json!({"count": 2, "subset": "lead-like"}),
        )
        .await
        .unwrap();
    let three_d = bio
        .call(
            "zinc_get_3d",
            &json!({"zinc_ids": ["ZINC99", "ZINC000000000077"]}),
        )
        .await
        .unwrap();
    server.abort();
    let traffic = captured.lock().unwrap().join("\n");
    assert!(traffic.contains("smiles smi=CCO"), "{traffic}");
    assert!(traffic.contains("sdist=2"), "{traffic}");
    assert!(traffic.contains("dist=1"), "{traffic}");
    assert!(traffic.contains("supplier_codes=SYNTH-0001"), "{traffic}");
    assert!(traffic.contains("random count=2"), "{traffic}");
    assert!(traffic.contains("subset=lead-like"), "{traffic}");
    assert_eq!(smiles["query"]["dist"], 2);
    assert_eq!(supplier["missing_ids"], json!(["MISSING-1"]));
    assert_eq!(random["query"]["subset"], "lead-like");
    assert_eq!(three_d["missing_ids"], json!(["ZINC000000000077"]));
    assert_eq!(three_d["structures"][0]["found"], true);
    assert_eq!(
        three_d["structures"][0]["download"]["repository"],
        "https://files.docking.org/zinc22/"
    );
    assert_eq!(
        three_d["structures"][0]["download"]["tranche_path_pattern"],
        "zinc-22*/H15/H15P090/"
    );
    assert_eq!(three_d["structures"][1]["found"], false);
    assert_eq!(three_d["files_url"], "https://files.docking.org/zinc22/");
}

#[tokio::test]
async fn polls_pending_tasks_until_success() {
    let polls = Arc::new(StdMutex::new(0u32));
    let count = polls.clone();
    let app = Router::new()
        .route(
            "/substances.txt",
            post(|| async { axum::Json(json!({"task": "pending-task"})) }),
        )
        .route(
            "/search/result/{task}",
            get(move |Path(task): Path<String>| {
                let count = count.clone();
                async move {
                    assert_eq!(task, "pending-task");
                    let mut n = count.lock().unwrap();
                    *n += 1;
                    if *n == 1 {
                        axum::Json(json!({"status": "PENDING", "result": []})).into_response()
                    } else {
                        axum::Json(json!({
                            "status": "SUCCESS",
                            "result": {"zinc22": [record()]}
                        }))
                        .into_response()
                    }
                }
            }),
        );
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "zinc_search_by_id",
            &json!({"zinc_ids": ["ZINC000000000099"]}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["returned"], 1);
    assert!(*polls.lock().unwrap() >= 2);
}

#[tokio::test]
async fn rejects_rate_limits_malformed_json_html_and_failed_tasks() {
    for (submit_status, submit_body, poll_status, poll_body, expected) in [
        (
            StatusCode::TOO_MANY_REQUESTS,
            "secret-token".into(),
            StatusCode::OK,
            json!({"status": "SUCCESS", "result": {}}).to_string(),
            "HTTP 429",
        ),
        (
            StatusCode::OK,
            json!({"task": "t"}).to_string(),
            StatusCode::OK,
            "{not-json".into(),
            "invalid JSON",
        ),
        (
            StatusCode::OK,
            "<!doctype html><html><body>app</body></html>".into(),
            StatusCode::OK,
            json!({"status": "SUCCESS"}).to_string(),
            "HTML app shell",
        ),
        (
            StatusCode::OK,
            json!({"task": "fail-task"}).to_string(),
            StatusCode::OK,
            json!({"status": "FAILURE"}).to_string(),
            "failed server-side",
        ),
    ] {
        let app = Router::new()
            .route(
                "/substances.txt",
                post({
                    let submit_body = submit_body.clone();
                    move || {
                        let submit_body = submit_body.clone();
                        async move {
                            (submit_status, [("retry-after", "60")], submit_body).into_response()
                        }
                    }
                }),
            )
            .route(
                "/search/result/{task}",
                get({
                    let poll_body = poll_body.clone();
                    move |Path(_task): Path<String>| {
                        let poll_body = poll_body.clone();
                        async move { (poll_status, poll_body).into_response() }
                    }
                }),
            );
        let (bio, server) = serve(app).await;
        let error = bio
            .call(
                "zinc_search_by_id",
                &json!({"zinc_ids": ["ZINC000000000099"]}),
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
async fn oversized_result_is_rejected_without_treating_empty_as_success() {
    let app = Router::new()
        .route(
            "/substances.txt",
            post(|| async { axum::Json(json!({"task": "big"})) }),
        )
        .route(
            "/search/result/{task}",
            get(|| async { (StatusCode::OK, " ".repeat(MAX_RESPONSE + 1)).into_response() }),
        );
    let (bio, server) = serve(app).await;
    let error = bio
        .call(
            "zinc_search_by_id",
            &json!({"zinc_ids": ["ZINC000000000099"]}),
        )
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("exceeded 4 MiB"), "{error}");
}

#[tokio::test]
async fn truncates_large_hit_lists_and_keeps_source_counts() {
    let hits: Vec<Value> = (0..3)
        .map(|i| {
            json!({
                "zinc_id": format!("ZINC{:012}", i + 1),
                "smiles": "C",
                "tranche_name": "H08P010"
            })
        })
        .collect();
    let _ = hits;
    let app = Router::new()
        .route("/search/maps", get(|| async { axum::Json(json!({"zinc20-forsale-test":{"enabled":true}})) }))
        .route("/search/view", get(|| async { axum::Json(json!({"status":{"state":"DONE"},"recordsFiltered":3,"data":[[{"id":"1","hitSmiles":"C 1"},0,0],[{"id":"2","hitSmiles":"C 2"},0,0],[{"id":"3","hitSmiles":"C 3"},0,0]]})) }));
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "zinc_search_by_smiles",
            &json!({"smiles": "C", "max_results": 2}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["total_available"], 3);
    assert_eq!(result["returned"], 2);
    assert_eq!(result["truncated"], true);
    assert_eq!(result["source_counts"]["zinc20"], 3);
}

#[tokio::test]
async fn unknown_tool_name_is_rejected() {
    let (bio, server) = serve(Router::new()).await;
    let error = call(&bio, "zinc_not_a_tool", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("unknown native biological tool"));
}
