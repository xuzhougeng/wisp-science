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
    let origin = base.trim_end_matches('/').to_string();
    NativeBio::test_client(
        &[
            ("ANTIBODY_REGISTRY_BASE_URL".into(), origin.clone()),
            ("GRANTS_GOV_BASE_URL".into(), origin),
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

fn antibody_item(id: u64, catalog: &str, vendor: &str) -> Value {
    json!({
        "abId": id,
        "abName": format!("Synthetic antibody {id}"),
        "abTarget": "TP53",
        "catalogNum": catalog,
        "catAlt": "ALT-1, ALT-2",
        "vendorName": vendor,
        "cloneId": "DO-1",
        "clonality": "monoclonal",
        "sourceOrganism": "Mouse",
        "targetSpecies": ["Human"],
        "url": "https://vendor.example.test/ab",
        "curateTime": "2024-01-01T00:00:00Z",
        "lastEditTime": "2024-02-01T00:00:00Z"
    })
}

fn grant_hit(id: &str, number: &str) -> Value {
    json!({
        "id": id,
        "number": number,
        "title": format!("Synthetic {number}"),
        "agencyCode": "HHS-NIH11",
        "agencyName": "National Institutes of Health",
        "openDate": "01/15/2026",
        "closeDate": "05/01/2026",
        "oppStatus": "posted",
        "docType": "synopsis",
        "alnist": ["93.866"]
    })
}

fn grants_envelope(hit_count: u64, hits: Vec<Value>) -> Value {
    json!({
        "errorcode": 0,
        "msg": "Webservice Succeeds",
        "token": "synthetic-token",
        "data": {
            "hitCount": hit_count,
            "startRecord": 0,
            "oppHits": hits,
            "oppStatusOptions": [{"label": "posted", "value": "posted", "count": hit_count}],
            "agencies": [{"label": "NIH", "value": "HHS-NIH11", "count": hit_count}],
            "eligibilities": [],
            "fundingCategories": [{"label": "Health", "value": "HL", "count": hit_count}],
            "fundingInstruments": [{"label": "Grant", "value": "G", "count": hit_count}]
        }
    })
}

#[test]
fn catalog_registers_five_research_resources_tools() {
    let names: Vec<_> = catalog()
        .into_iter()
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        names,
        vec![
            ("research-resources", "search_antibodies".into()),
            ("research-resources", "get_antibody".into()),
            ("research-resources", "find_antibodies_by_catalog".into()),
            ("research-resources", "get_antibody_registry_stats".into()),
            ("research-resources", "search_grants".into()),
        ]
    );
    assert!(crate::contains_tool("search_antibodies"));
    assert_eq!(
        crate::domain_for_tool("search_grants"),
        Some("research-resources")
    );
    assert!(crate::package_selects(
        "mcp_research_resources",
        "research-resources"
    ));
    assert!(crate::selected_by_package("mcp_research_resources"));
}

#[test]
fn antibody_arguments_are_bounded() {
    assert_eq!(antibodies::parse_ab_id("3643095").unwrap(), 3643095);
    assert_eq!(antibodies::parse_ab_id("RRID:AB_3643095").unwrap(), 3643095);
    assert_eq!(antibodies::parse_ab_id("ab_12").unwrap(), 12);
    for value in [
        "",
        "0",
        "AB_",
        "RRID:AB_0",
        "pmid:123",
        "03643095",
        "not-an-id",
    ] {
        assert!(antibodies::parse_ab_id(value).is_err(), "{value}");
    }
    assert!(serde_json::from_value::<antibodies::Search>(
        json!({"query": "TP53", "api_key": "secret"})
    )
    .is_err());
    assert!(
        serde_json::from_value::<antibodies::Get>(json!({"antibody_id": "1", "token": "x"}))
            .is_err()
    );
    assert!(serde_json::from_value::<grants::Search>(
        json!({"keyword": "ALS", "api_key": "secret"})
    )
    .is_err());
}

#[tokio::test]
async fn invalid_arguments_are_rejected_before_http() {
    let bio = NativeBio::new(&[]).unwrap();
    for (name, args) in [
        ("search_antibodies", json!({"query": " "})),
        (
            "search_antibodies",
            json!({"query": "TP53", "page": 6, "page_size": 100}),
        ),
        (
            "search_antibodies",
            json!({"query": "TP53", "max_records": 501}),
        ),
        ("get_antibody", json!({"antibody_id": "pmid:123"})),
        ("find_antibodies_by_catalog", json!({"catalog_number": " "})),
        ("search_grants", json!({})),
        ("search_grants", json!({"keyword": " "})),
        ("search_grants", json!({"opportunity_statuses": ["posted"]})),
        (
            "search_grants",
            json!({"keyword": "ALS", "opportunity_statuses": ["live"]}),
        ),
        (
            "search_grants",
            json!({"keyword": "ALS", "agencies": ["HHS|NIH"]}),
        ),
        (
            "search_grants",
            json!({"keyword": "ALS", "max_records": 201}),
        ),
    ] {
        let error = bio.call(name, &args).await.unwrap_err().to_string();
        assert!(
            !error.contains("connection failed"),
            "{name} {args} {error}"
        );
    }
}

#[tokio::test]
async fn search_antibodies_dispatches_fts_query_and_reports_source_urls() {
    let captured = Arc::new(StdMutex::new(HashMap::new()));
    let seen = captured.clone();
    let app = Router::new().route(
        "/api/fts-antibodies",
        get(move |Query(params): Query<HashMap<String, String>>| {
            *seen.lock().unwrap() = params.clone();
            let page: u32 = params.get("page").and_then(|v| v.parse().ok()).unwrap_or(1);
            async move {
                axum::Json(json!({
                    "page": page,
                    "totalElements": 2,
                    "items": [
                        antibody_item(3643095, "ab32572", "Abcam"),
                        antibody_item(3643095, "ab32572", "Abcam")
                    ]
                }))
            }
        }),
    );
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "search_antibodies",
            &json!({"query": "TP53", "page": 1, "page_size": 50}),
        )
        .await
        .unwrap();
    server.abort();
    let params = captured.lock().unwrap().clone();
    assert_eq!(params.get("q").map(String::as_str), Some("TP53"));
    assert_eq!(params.get("page").map(String::as_str), Some("1"));
    assert_eq!(params.get("size").map(String::as_str), Some("50"));
    assert_eq!(result["source"], "Antibody Registry");
    assert_eq!(result["source_url"], ANTIBODY_SITE);
    assert_eq!(result["total_elements"], 2);
    assert_eq!(result["returned"], 2);
    assert_eq!(result["unique_ab_ids"], 1);
    assert_eq!(result["has_more"], false);
    assert_eq!(
        result["records"][0]["registry_url"],
        "https://www.antibodyregistry.org/AB_3643095"
    );
    assert_eq!(result["records"][0]["rrid"], "AB_3643095");
    assert!(result["records"][0].get("curateTime").is_none());
}

#[tokio::test]
async fn antibody_walk_stops_at_anonymous_offset_cap() {
    let pages = Arc::new(StdMutex::new(Vec::<u32>::new()));
    let seen = pages.clone();
    let app = Router::new().route(
        "/api/fts-antibodies",
        get(move |Query(params): Query<HashMap<String, String>>| {
            let page: u32 = params.get("page").and_then(|v| v.parse().ok()).unwrap_or(1);
            seen.lock().unwrap().push(page);
            async move {
                if page >= 6 {
                    return (StatusCode::UNAUTHORIZED, "secret-token").into_response();
                }
                axum::Json(json!({
                    "page": page,
                    "totalElements": 600,
                    "items": [antibody_item(1000 + page as u64, "CAT", "Vendor")]
                }))
                .into_response()
            }
        }),
    );
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "search_antibodies",
            &json!({"query": "p53", "page_size": 100, "max_records": 500}),
        )
        .await
        .unwrap();
    server.abort();
    let requested = pages.lock().unwrap().clone();
    assert_eq!(requested, vec![1, 2, 3, 4, 5]);
    assert_eq!(result["anonymous_limit_hit"], true);
    assert_eq!(result["truncated"], true);
    assert_eq!(result["complete"], false);
    assert_eq!(result["returned"], 5);
    assert_eq!(result["total_elements"], 600);
}

#[tokio::test]
async fn get_antibody_treats_empty_list_as_missing_and_accepts_rrid() {
    let app = Router::new()
        .route(
            "/api/antibodies/{id}",
            get(|Path(id): Path<String>| async move {
                if id == "3643095" {
                    axum::Json(json!([antibody_item(3643095, "ab32572", "Abcam")])).into_response()
                } else {
                    axum::Json(json!([])).into_response()
                }
            }),
        )
        .route(
            "/api/datainfo",
            get(|| async { axum::Json(json!({"total": 3200000, "lastupdate": "2026-01-15"})) }),
        );
    let (bio, server) = serve(app).await;
    let found = bio
        .call("get_antibody", &json!({"antibody_id": "RRID:AB_3643095"}))
        .await
        .unwrap();
    let missing = bio
        .call("get_antibody", &json!({"antibody_id": "9999999"}))
        .await
        .unwrap();
    let stats = bio
        .call("get_antibody_registry_stats", &json!({}))
        .await
        .unwrap();
    server.abort();
    assert_eq!(found["found"], true);
    assert_eq!(found["returned"], 1);
    assert_eq!(found["rrid"], "AB_3643095");
    assert_eq!(missing["found"], false);
    assert_eq!(missing["returned"], 0);
    assert_eq!(stats["total_antibodies"], 3200000);
    assert_eq!(stats["last_update"], "2026-01-15");
    assert_eq!(stats["source_url"], ANTIBODY_SITE);
}

#[tokio::test]
async fn catalog_lookup_exact_matches_catalog_and_vendor() {
    let mut alternate = antibody_item(3, "other", "Cell Signaling");
    alternate["catAlt"] = json!("ab32572; ZZ-9");
    let items = json!({
        "page": 1,
        "totalElements": 3,
        "items": [
            antibody_item(1, "ab32572", "Abcam"),
            antibody_item(2, "ab32572-extra", "Abcam"),
            alternate
        ]
    });
    let app = Router::new().route(
        "/api/fts-antibodies",
        get(move || {
            let items = items.clone();
            async move { axum::Json(items) }
        }),
    );
    let (bio, server) = serve(app).await;
    let all = bio
        .call(
            "find_antibodies_by_catalog",
            &json!({"catalog_number": "AB32572"}),
        )
        .await
        .unwrap();
    let vendor = bio
        .call(
            "find_antibodies_by_catalog",
            &json!({"catalog_number": "ab32572", "vendor": "abcam"}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(all["returned"], 2);
    assert_eq!(all["records"][0]["ab_id"], 1);
    assert_eq!(all["records"][1]["ab_id"], 3);
    assert_eq!(vendor["returned"], 1);
    assert_eq!(vendor["records"][0]["vendor"], "Abcam");
}

#[tokio::test]
async fn search_grants_posts_json_and_reports_source_urls_and_truncation() {
    let captured = Arc::new(StdMutex::new(Value::Null));
    let body = captured.clone();
    let app = Router::new().route(
        "/v1/api/search2",
        post(move |incoming: String| {
            *body.lock().unwrap() = serde_json::from_str(&incoming).unwrap();
            let hits = vec![
                grant_hit("360400", "RFA-NS-26-001"),
                grant_hit("360401", "PAR-25-327"),
                grant_hit("360402", "PA-25-100"),
            ];
            async move { axum::Json(grants_envelope(8, hits)) }
        }),
    );
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "search_grants",
            &json!({
                "keyword": "ALS",
                "agencies": ["HHS-NIH11", "NSF"],
                "aln": "93.866",
                "max_records": 2
            }),
        )
        .await
        .unwrap();
    server.abort();
    let payload = captured.lock().unwrap().clone();
    assert_eq!(payload["keyword"], "ALS");
    assert_eq!(payload["agencies"], "HHS-NIH11|NSF");
    assert_eq!(payload["aln"], "93.866");
    assert_eq!(payload["oppStatuses"], "forecasted|posted");
    assert_eq!(payload["rows"], 2);
    assert_eq!(payload["startRecordNum"], 0);
    assert!(payload.get("token").is_none());
    assert_eq!(result["source"], "Grants.gov");
    assert_eq!(result["source_url"], GRANTS_SITE);
    assert_eq!(result["total"], 8);
    assert_eq!(result["returned"], 2);
    assert_eq!(result["truncated"], true);
    assert_eq!(result["has_more"], true);
    assert_eq!(result["next_start_record"], 2);
    assert_eq!(
        result["records"][0]["url"],
        "https://www.grants.gov/search-results-detail/360400"
    );
    assert_eq!(
        result["records"][0]["assistance_listings"],
        json!(["93.866"])
    );
    assert!(result.get("token").is_none());
    assert_eq!(result["facets"]["fundingInstruments"][0]["value"], "G");
}

#[tokio::test]
async fn search_grants_count_only_omits_records_and_uses_rows_zero() {
    let captured = Arc::new(StdMutex::new(Value::Null));
    let body = captured.clone();
    let app = Router::new().route(
        "/v1/api/search2",
        post(move |incoming: String| {
            *body.lock().unwrap() = serde_json::from_str(&incoming).unwrap();
            async move { axum::Json(grants_envelope(12, vec![])) }
        }),
    );
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "search_grants",
            &json!({
                "opportunity_number": "PAR-25-327",
                "count_only": true,
                "include_facets": false
            }),
        )
        .await
        .unwrap();
    server.abort();
    let payload = captured.lock().unwrap().clone();
    assert_eq!(payload["rows"], 0);
    assert_eq!(payload["oppNum"], "PAR-25-327");
    assert_eq!(result["total"], 12);
    assert_eq!(result["returned"], 0);
    assert!(result.get("facets").is_none());
}

#[tokio::test]
async fn rejects_rate_limits_malformed_json_and_upstream_envelopes_without_echoing_secrets() {
    for (path, method_post, status, body, call_name, args, expected) in [
        (
            "/api/fts-antibodies",
            false,
            StatusCode::TOO_MANY_REQUESTS,
            "secret-token".into(),
            "search_antibodies",
            json!({"query": "TP53"}),
            "HTTP 429",
        ),
        (
            "/api/fts-antibodies",
            false,
            StatusCode::OK,
            "{not-json".into(),
            "search_antibodies",
            json!({"query": "TP53"}),
            "invalid JSON",
        ),
        (
            "/v1/api/search2",
            true,
            StatusCode::TOO_MANY_REQUESTS,
            "secret-token".into(),
            "search_grants",
            json!({"keyword": "ALS"}),
            "HTTP 429",
        ),
        (
            "/v1/api/search2",
            true,
            StatusCode::OK,
            json!({"errorcode": 1, "msg": "secret-token", "data": {}}).to_string(),
            "search_grants",
            json!({"keyword": "ALS"}),
            "rejected",
        ),
    ] {
        let app = if method_post {
            Router::new().route(
                path,
                post({
                    let body = body.clone();
                    move || {
                        let body = body.clone();
                        async move { (status, [("retry-after", "60")], body).into_response() }
                    }
                }),
            )
        } else {
            Router::new().route(
                path,
                get({
                    let body = body.clone();
                    move || {
                        let body = body.clone();
                        async move { (status, [("retry-after", "60")], body).into_response() }
                    }
                }),
            )
        };
        let (bio, server) = serve(app).await;
        let error = bio.call(call_name, &args).await.unwrap_err().to_string();
        server.abort();
        assert!(
            error.contains(expected),
            "{error} did not contain {expected}"
        );
        assert!(!error.contains("secret-token"), "{error}");
    }
}

#[tokio::test]
async fn oversized_grants_response_is_rejected() {
    let app = Router::new().route(
        "/v1/api/search2",
        post(|| async { (StatusCode::OK, " ".repeat(MAX_RESPONSE + 1)).into_response() }),
    );
    let (bio, server) = serve(app).await;
    let error = bio
        .call("search_grants", &json!({"keyword": "ALS"}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("exceeded 4 MiB"), "{error}");
}

#[tokio::test]
async fn missing_totals_and_empty_success_are_distinguished() {
    let app = Router::new()
        .route(
            "/api/fts-antibodies",
            get(|| async { axum::Json(json!({"page": 1, "items": []})) }),
        )
        .route(
            "/v1/api/search2",
            post(|| async {
                axum::Json(json!({
                    "errorcode": 0,
                    "data": {"oppHits": []}
                }))
            }),
        );
    let (bio, server) = serve(app).await;
    let antibody = bio
        .call("search_antibodies", &json!({"query": "no-such-target"}))
        .await
        .unwrap_err()
        .to_string();
    let grants = bio
        .call("search_grants", &json!({"keyword": "no-such-grant"}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(antibody.contains("totalElements"), "{antibody}");
    assert!(grants.contains("hitCount"), "{grants}");
}

#[tokio::test]
async fn empty_hits_are_success_when_upstream_total_is_zero() {
    let app = Router::new()
        .route(
            "/api/fts-antibodies",
            get(|| async { axum::Json(json!({"page": 1, "totalElements": 0, "items": []})) }),
        )
        .route(
            "/v1/api/search2",
            post(|| async { axum::Json(grants_envelope(0, vec![])) }),
        );
    let (bio, server) = serve(app).await;
    let antibodies = bio
        .call("search_antibodies", &json!({"query": "no-such-target"}))
        .await
        .unwrap();
    let grants = bio
        .call("search_grants", &json!({"keyword": "no-such-grant"}))
        .await
        .unwrap();
    server.abort();
    assert_eq!(antibodies["returned"], 0);
    assert_eq!(antibodies["total_elements"], 0);
    assert_eq!(grants["returned"], 0);
    assert_eq!(grants["total"], 0);
    assert_eq!(grants["has_more"], false);
}

#[tokio::test]
async fn unknown_tool_name_is_rejected() {
    let (bio, server) = serve(Router::new()).await;
    let error = call(&bio, "not_a_research_resource", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("unknown native biological tool"));
}

#[tokio::test]
async fn single_page_beyond_anonymous_window_is_rejected_without_http() {
    let (bio, server) = serve(Router::new()).await;
    let error = bio
        .call(
            "search_antibodies",
            &json!({"query": "TP53", "page": 6, "page_size": 100}),
        )
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(
        error.contains("401") || error.contains("offset cap"),
        "{error}"
    );
}
