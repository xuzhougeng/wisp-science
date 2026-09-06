use super::*;
use crate::http::{Http, MAX_RESPONSE};
use axum::{
    http::{StatusCode, Uri},
    response::IntoResponse,
    routing::get,
    Router,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

fn test_bio(base: &str, key: Option<&str>) -> NativeBio {
    let mut credentials = vec![("OPENFDA_BASE_URL".into(), base.trim_end_matches('/').into())];
    if let Some(key) = key {
        credentials.push(("OPENFDA_API_KEY".into(), key.into()));
    }
    NativeBio::test_client(
        &credentials,
        Http(reqwest::Client::builder().no_proxy().build().unwrap()),
    )
    .unwrap()
}

async fn serve(app: Router) -> (NativeBio, tokio::task::JoinHandle<()>) {
    serve_with_key(app, Some("synthetic-key&value")).await
}

async fn serve_with_key(
    app: Router,
    key: Option<&str>,
) -> (NativeBio, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (test_bio(&endpoint, key), task)
}

fn application_doc(number: &str, brand: &str, ingredient: &str) -> Value {
    json!({
        "application_number": number,
        "sponsor_name": "SYNTHETIC LABS",
        "products": [{
            "product_number": "001",
            "brand_name": brand,
            "dosage_form": "TABLET",
            "route": "ORAL",
            "marketing_status": "Prescription",
            "te_code": "AB",
            "reference_drug": "Yes",
            "reference_standard": "Yes",
            "active_ingredients": [{"name": ingredient, "strength": "10 mg"}]
        }],
        "submissions": [{
            "submission_type": "ORIG",
            "submission_number": "1",
            "submission_status": "AP",
            "submission_status_date": "20200115"
        }],
        "openfda": {
            "generic_name": [ingredient],
            "brand_name": [brand],
            "pharm_class_epc": ["Synthetic Diuretic [EPC]"]
        }
    })
}

fn label_doc() -> Value {
    json!({
        "set_id": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
        "id": "11111111-2222-3333-4444-555555555555",
        "version": "8",
        "effective_time": "20240101",
        "openfda": {
            "brand_name": ["SYNTHAPRIL"],
            "generic_name": ["SYNTHAPRILUM"],
            "substance_name": ["SYNTHAPRILUM"],
            "manufacturer_name": ["SYNTHETIC LABS"],
            "route": ["ORAL"],
            "product_type": ["HUMAN PRESCRIPTION DRUG"],
            "application_number": ["NDA000001"]
        },
        "boxed_warning": ["Do not use in the fictional syndrome."],
        "indications_and_usage": ["For simulated hypertension."],
        "dosage_and_administration": ["One tablet daily."]
    })
}

fn wrap(total: u64, results: Vec<Value>) -> Value {
    json!({
        "meta": {
            "last_updated": "2026-01-15",
            "results": {"skip": 0, "limit": results.len(), "total": total}
        },
        "results": results
    })
}

fn query_from_uri(uri: &Uri) -> HashMap<String, String> {
    reqwest::Url::parse(&format!("http://openfda.test{}", uri))
        .map(|url| {
            url.query_pairs()
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect()
        })
        .unwrap_or_default()
}

fn capture_router(
    captured: Arc<StdMutex<Vec<String>>>,
    drugsfda: Arc<dyn Fn(HashMap<String, String>) -> axum::response::Response + Send + Sync>,
    labels: Arc<dyn Fn(HashMap<String, String>) -> axum::response::Response + Send + Sync>,
) -> Router {
    let drugs_log = captured.clone();
    let drugs_fn = drugsfda.clone();
    let labels_log = captured;
    let labels_fn = labels;
    Router::new()
        .route(
            "/drug/drugsfda.json",
            get(move |uri: Uri| {
                let captured = drugs_log.clone();
                let drugsfda = drugs_fn.clone();
                async move {
                    captured.lock().unwrap().push(uri.to_string());
                    drugsfda(query_from_uri(&uri))
                }
            }),
        )
        .route(
            "/drug/label.json",
            get(move |uri: Uri| {
                let captured = labels_log.clone();
                let labels = labels_fn.clone();
                async move {
                    captured.lock().unwrap().push(uri.to_string());
                    labels(query_from_uri(&uri))
                }
            }),
        )
}

#[test]
fn catalog_registers_seven_drug_regulatory_tools() {
    let names: Vec<_> = catalog()
        .into_iter()
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        names,
        vec![
            ("drug-regulatory", "count_drug_applications".into()),
            ("drug-regulatory", "get_drug_application".into()),
            ("drug-regulatory", "get_drug_statistics".into()),
            ("drug-regulatory", "get_generic_equivalents".into()),
            ("drug-regulatory", "list_pharmacologic_classes".into()),
            ("drug-regulatory", "search_drug_applications".into()),
            ("drug-regulatory", "search_drug_labels".into()),
        ]
    );
    assert!(crate::contains_tool("search_drug_applications"));
    assert_eq!(
        crate::domain_for_tool("search_drug_applications"),
        Some("drug-regulatory")
    );
    assert!(crate::package_selects(
        "mcp_drug_regulatory",
        "drug-regulatory"
    ));
    assert!(crate::selected_by_package("mcp_drug_regulatory"));
}

#[test]
fn rejects_unbounded_or_malformed_arguments() {
    for args in [
        json!({}),
        json!({"brand": " "}),
        json!({"brand": "SYNTHAPRIL", "max_records": 0}),
        json!({"brand": "SYNTHAPRIL", "max_records": 101}),
        json!({"brand": "SYNTHAPRIL", "skip": 25001}),
        json!({"brand": "SYNTHAPRIL", "search_type": "xor"}),
        json!({"brand": "SYNTHAPRIL", "pharm_class_type": "atc"}),
        json!({"brand": "SYNTHAPRIL", "submission_date_from": "2020-01-01"}),
        json!({"brand": "SYNTHAPRIL", "submission_date_from": "20201301", "submission_date_to": "20200101"}),
        json!({"raw_search": "sponsor_name:X", "brand": "SYNTHAPRIL"}),
        json!({"brand": "SYNTHAPRIL", "api_key": "secret"}),
    ] {
        match serde_json::from_value::<SearchApplications>(args.clone()) {
            Ok(parsed) => assert!(
                application_search(&parsed).is_err()
                    || bound(parsed.max_records, 1, APP_PAGE, "max_records").is_err()
                    || bound_skip(parsed.skip).is_err(),
                "{args}"
            ),
            Err(_) => {}
        }
    }
    assert!(normalize_application_number("NDA20").is_err());
    assert!(normalize_application_number("NDA020702X").is_err());
    assert_eq!(
        normalize_application_number(" anda076543 ").unwrap(),
        "ANDA076543"
    );
    assert!(resolve_count_field("not_a_field").is_err());
    assert_eq!(
        resolve_count_field("dosage_form").unwrap(),
        "products.dosage_form.exact"
    );
    assert!(serde_json::from_value::<GetApplication>(
        json!({"application_number": "NDA000001", "api_key": "x"})
    )
    .is_err());
}

#[test]
fn search_strings_quote_phrases_join_filters_and_normalize_dates() {
    let args: SearchApplications = serde_json::from_value(json!({
        "brand": "SYNTHAPRIL FORTE",
        "sponsor": "SYNTHETIC LABS",
        "pharm_class": "Synthetic Diuretic [EPC]",
        "search_type": "or",
        "submission_date_from": "2020-01-02",
        "submission_date_to": "20201231"
    }))
    .unwrap();
    let search = application_search(&args).unwrap();
    assert!(search.starts_with('('), "{search}");
    assert!(
        search.contains("products.brand_name:\"SYNTHAPRIL FORTE\""),
        "{search}"
    );
    assert!(search.contains(" OR "), "{search}");
    assert!(
        search.contains("openfda.pharm_class_epc:\"Synthetic Diuretic [EPC]\""),
        "{search}"
    );
    assert!(
        search.contains("submissions.submission_status_date:[20200102 TO 20201231]"),
        "{search}"
    );
    let labels: SearchLabels = serde_json::from_value(json!({
        "brand_name": "SYNTHAPRIL",
        "exact": true
    }))
    .unwrap();
    assert_eq!(
        label_search(&labels).unwrap(),
        "openfda.brand_name.exact:\"SYNTHAPRIL\""
    );
}

#[test]
fn application_projection_adds_source_urls_and_iso_dates() {
    let record = project_application(
        &application_doc("NDA000001", "SYNTHAPRIL", "SYNTHAPRILUM"),
        true,
    )
    .unwrap();
    assert_eq!(record["application_number"], "NDA000001");
    assert_eq!(
        record["url"],
        "https://api.fda.gov/drug/drugsfda.json?search=application_number:\"NDA000001\""
    );
    assert!(record["fda_url"]
        .as_str()
        .unwrap()
        .contains("ApplNo=000001"));
    assert_eq!(
        record["submissions"][0]["submission_status_date"],
        "2020-01-15"
    );
    assert_eq!(record["openfda"]["generic_name"], json!(["SYNTHAPRILUM"]));
    let label = project_label(&label_doc(), None);
    assert_eq!(label["has_boxed_warning"], true);
    assert_eq!(label["warning_sections_present"], json!(["boxed_warning"]));
    assert!(label["dailymed_url"]
        .as_str()
        .unwrap()
        .contains("setid=aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"));
}

#[tokio::test]
async fn search_applications_pages_and_keeps_source_urls_without_echoing_keys() {
    let captured = Arc::new(StdMutex::new(Vec::new()));
    let app = capture_router(
        captured.clone(),
        Arc::new(|_params| {
            axum::Json(wrap(
                3,
                vec![
                    application_doc("NDA000001", "SYNTHAPRIL", "SYNTHAPRILUM"),
                    application_doc("ANDA000002", "SYNTHAPRIL", "SYNTHAPRILUM"),
                ],
            ))
            .into_response()
        }),
        Arc::new(|_params| StatusCode::NOT_FOUND.into_response()),
    );
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "search_drug_applications",
            &json!({"brand": "SYNTHAPRIL", "max_records": 2, "skip": 0}),
        )
        .await
        .unwrap();
    server.abort();
    let traffic = captured.lock().unwrap().join("\n");
    assert!(traffic.contains("search="), "{traffic}");
    assert!(
        traffic.contains("api_key=synthetic-key%26value"),
        "{traffic}"
    );
    assert!(traffic.contains("limit=2"), "{traffic}");
    assert_eq!(result["source"], "openFDA Drugs@FDA");
    assert_eq!(result["source_url"], DRUGSFDA_DOCS);
    assert_eq!(result["total"], 3);
    assert_eq!(result["returned"], 2);
    assert_eq!(result["truncated"], true);
    assert_eq!(result["has_more"], true);
    assert_eq!(result["next_skip"], 2);
    assert!(result["query_url"]
        .as_str()
        .unwrap()
        .starts_with("https://api.fda.gov/drug/drugsfda.json"));
    assert!(!result["query_url"].as_str().unwrap().contains("api_key"));
    assert!(!result.to_string().contains("synthetic-key"));
    assert_eq!(
        result["records"][0]["url"],
        "https://api.fda.gov/drug/drugsfda.json?search=application_number:\"NDA000001\""
    );
}

#[tokio::test]
async fn remaining_tools_dispatch_through_native_bio_call() {
    let captured = Arc::new(StdMutex::new(Vec::new()));
    let app = capture_router(
        captured.clone(),
        Arc::new(|params| {
            if params.get("count").is_some() {
                return axum::Json(json!({
                    "meta": {"last_updated": "2026-01-15"},
                    "results": [
                        {"term": "Prescription", "count": 9},
                        {"term": "TABLET", "count": 4},
                        {"term": "ORAL", "count": 4},
                        {"term": "SYNTHETIC LABS", "count": 3},
                        {"term": "Synthetic Diuretic [EPC]", "count": 2}
                    ]
                }))
                .into_response();
            }
            let search = params.get("search").cloned().unwrap_or_default();
            if search.contains("application_number") {
                return axum::Json(wrap(
                    1,
                    vec![application_doc("NDA000001", "SYNTHAPRIL", "SYNTHAPRILUM")],
                ))
                .into_response();
            }
            if search.contains("products.brand_name") {
                return axum::Json(wrap(
                    1,
                    vec![application_doc("NDA000001", "SYNTHAPRIL", "SYNTHAPRILUM")],
                ))
                .into_response();
            }
            if search.contains("products.active_ingredients.name") {
                return axum::Json(wrap(
                    2,
                    vec![
                        application_doc("NDA000001", "SYNTHAPRIL", "SYNTHAPRILUM"),
                        application_doc("ANDA000002", "SYNTHAPRIL-G", "SYNTHAPRILUM"),
                    ],
                ))
                .into_response();
            }
            axum::Json(wrap(
                12,
                vec![application_doc("NDA000001", "SYNTHAPRIL", "SYNTHAPRILUM")],
            ))
            .into_response()
        }),
        Arc::new(|_params| axum::Json(wrap(1, vec![label_doc()])).into_response()),
    );
    let (bio, server) = serve(app).await;
    let got = bio
        .call(
            "get_drug_application",
            &json!({"application_number": "nda000001"}),
        )
        .await
        .unwrap();
    let counted = bio
        .call(
            "count_drug_applications",
            &json!({"count_field": "sponsor_name", "brand": "SYNTHAPRIL", "max_buckets": 5}),
        )
        .await
        .unwrap();
    let stats = bio.call("get_drug_statistics", &json!({})).await.unwrap();
    let classes = bio
        .call(
            "list_pharmacologic_classes",
            &json!({"class_type": "epc", "max_buckets": 5}),
        )
        .await
        .unwrap();
    let equiv = bio
        .call(
            "get_generic_equivalents",
            &json!({"brand": "SYNTHAPRIL", "max_records": 10}),
        )
        .await
        .unwrap();
    let labels = bio
        .call(
            "search_drug_labels",
            &json!({"brand_name": "SYNTHAPRIL", "sections": ["boxed_warning", "dosage_and_administration"]}),
        )
        .await
        .unwrap();
    server.abort();
    let traffic = captured.lock().unwrap().join("\n");
    assert!(traffic.contains("/drug/drugsfda.json"), "{traffic}");
    assert!(traffic.contains("/drug/label.json"), "{traffic}");
    assert!(traffic.contains("count=sponsor_name"), "{traffic}");
    assert!(
        traffic.contains("count=openfda.pharm_class_epc.exact"),
        "{traffic}"
    );
    assert_eq!(got["found"], true);
    assert_eq!(got["record"]["application_number"], "NDA000001");
    assert_eq!(
        got["fda_url"].as_str().unwrap().contains("ApplNo=000001"),
        true
    );
    assert_eq!(counted["api_field"], "sponsor_name");
    assert_eq!(counted["returned"], 5);
    assert_eq!(stats["total_applications"], 12);
    assert_eq!(stats["source_url"], DRUGSFDA_DOCS);
    assert_eq!(classes["class_type"], "epc");
    assert_eq!(equiv["reference_applications"], json!(["NDA000001"]));
    assert_eq!(equiv["returned"], 2);
    assert_eq!(labels["source"], "openFDA drug label");
    assert_eq!(
        labels["records"][0]["boxed_warning"],
        "Do not use in the fictional syndrome."
    );
    assert!(labels["records"][0]["dailymed_url"]
        .as_str()
        .unwrap()
        .contains("setid="));
    assert!(!equiv.to_string().contains("synthetic-key"));
}

#[tokio::test]
async fn missing_matches_are_empty_not_errors() {
    let app = capture_router(
        Arc::new(StdMutex::new(Vec::new())),
        Arc::new(|_params| {
            (
                StatusCode::NOT_FOUND,
                axum::Json(json!({"error": {"code": "NOT_FOUND", "message": "No matches found!"}})),
            )
                .into_response()
        }),
        Arc::new(|_params| StatusCode::NOT_FOUND.into_response()),
    );
    let (bio, server) = serve(app).await;
    let search = bio
        .call(
            "search_drug_applications",
            &json!({"brand": "NO-SUCH-DRUG"}),
        )
        .await
        .unwrap();
    let got = bio
        .call(
            "get_drug_application",
            &json!({"application_number": "NDA000099"}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(search["total"], 0);
    assert_eq!(search["returned"], 0);
    assert_eq!(search["has_more"], false);
    assert_eq!(got["found"], false);
    assert_eq!(got["record"], Value::Null);
}

#[tokio::test]
async fn rejects_rate_limits_malformed_json_and_html_without_echoing_secrets() {
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
            json!({"error": {"code": "BAD_REQUEST", "message": "secret-token"}}).to_string(),
            "rejected",
        ),
    ] {
        let app = Router::new().route(
            "/drug/drugsfda.json",
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
            .call("search_drug_applications", &json!({"brand": "SYNTHAPRIL"}))
            .await
            .unwrap_err()
            .to_string();
        server.abort();
        assert!(
            error.contains(expected),
            "{error} did not contain {expected}"
        );
        assert!(!error.contains("secret-token"), "{error}");
        assert!(!error.contains("synthetic-key"), "{error}");
    }
}

#[tokio::test]
async fn oversized_result_is_rejected() {
    let app = Router::new().route(
        "/drug/drugsfda.json",
        get(|| async { (StatusCode::OK, " ".repeat(MAX_RESPONSE + 1)).into_response() }),
    );
    let (bio, server) = serve(app).await;
    let error = bio
        .call("search_drug_applications", &json!({"brand": "SYNTHAPRIL"}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("exceeded 4 MiB"), "{error}");
}

#[tokio::test]
async fn unknown_tool_name_is_rejected() {
    let (bio, server) = serve(Router::new()).await;
    let error = call(&bio, "not_a_drug_tool", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("unknown native biological tool"));
}
