use super::*;
use crate::http::{Http, MAX_RESPONSE};
use crate::NativeBio;
use axum::{
    http::{StatusCode, Uri},
    response::IntoResponse,
    Router,
};
use serde_json::json;
use std::sync::{Arc, Mutex as StdMutex};

#[test]
fn catalog_registers_the_content_api_tools() {
    let expected = [
        "get_categories",
        "get_content_statistics",
        "get_preprint",
        "get_usage_statistics",
        "search_by_funder",
        "search_preprints",
        "search_published_preprints",
    ];
    let tools: Vec<_> = crate::catalog()
        .into_iter()
        .filter(|(domain, _)| *domain == "biorxiv")
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        tools,
        expected
            .iter()
            .map(|name| ("biorxiv", (*name).to_string()))
            .collect::<Vec<_>>()
    );
    assert_eq!(crate::domain_for_tool("search_preprints"), Some("biorxiv"));
}

#[test]
fn validates_identifiers_intervals_and_bounds() {
    assert!(normalize_doi("https://doi.org/10.1101/2024.01.01.999999").is_ok());
    assert!(normalize_doi("10.1101/2024.01.01.999999").is_ok());
    assert!(normalize_doi(" ").is_err());
    assert!(normalize_doi("10.1101").is_err());
    assert!(normalize_ror("https://ror.org/021nxhr62").unwrap() == "021nxhr62");
    assert!(normalize_ror("021nxhr62").is_ok());
    assert!(normalize_ror("too-short").is_err());
    assert!(listing_interval(None, None, None, None)
        .unwrap()
        .path()
        .eq("60d"));
    assert_eq!(
        listing_interval(Some("2024-01-01"), Some("2024-01-31"), None, None)
            .unwrap()
            .path(),
        "2024-01-01/2024-01-31"
    );
    assert_eq!(
        listing_interval(None, None, Some(30), None).unwrap().path(),
        "30d"
    );
    assert_eq!(
        listing_interval(None, None, None, Some(50)).unwrap().path(),
        "50"
    );
    assert!(listing_interval(Some("2024-01-01"), None, None, None).is_err());
    assert!(listing_interval(Some("2024-02-01"), Some("2024-01-01"), None, None).is_err());
    assert!(listing_interval(Some("2024-01-32"), Some("2024-02-01"), None, None).is_err());
    assert!(listing_interval(None, None, Some(30), Some(10)).is_err());
    assert!(listing_interval(None, None, Some(0), None).is_err());
    assert_eq!(
        category_param(Some("Cancer Biology")).unwrap().as_deref(),
        Some("cancer_biology")
    );
    assert!(category_param(Some("not a/category")).is_err());
    assert!(publisher_prefix(Some("10.1038"), "biorxiv")
        .unwrap()
        .is_some());
    assert!(publisher_prefix(Some("10.1038"), "medrxiv").is_err());
    assert!(publisher_prefix(Some("10.1038/nature"), "biorxiv").is_err());
    assert!(serde_json::from_value::<Search>(json!({"api_key": "secret"})).is_err());
    assert!(serde_json::from_value::<Search>(json!({"limit": 0})).is_ok());
    assert!(listing(&serde_json::from_value::<Search>(json!({"limit": 0})).unwrap()).is_err());
    assert!(listing(&serde_json::from_value::<Search>(json!({"limit": 101})).unwrap()).is_err());
}

#[test]
fn categories_are_local_underscore_forms() {
    let result = categories(&json!({})).unwrap();
    assert_eq!(result["source"], "bioRxiv Content API");
    assert_eq!(result["source_url"], SOURCE_URL);
    assert_eq!(result["returned"], 27);
    assert_eq!(result["categories"][5]["name"], "cancer biology");
    assert_eq!(result["categories"][5]["api_format"], "cancer_biology");
    assert!(categories(&json!({"unexpected": true})).is_err());
}

#[test]
fn summaries_truncate_abstracts_and_pick_the_latest_version() {
    let (preview, truncated) = abstract_preview(&"α".repeat(ABSTRACT_PREVIEW + 1));
    assert!(truncated);
    assert_eq!(preview.as_str().unwrap().chars().count(), ABSTRACT_PREVIEW);
    let versions = vec![
        json!({"doi": "10.1101/2024.01.01.999999", "version": "1", "title": "older"}),
        json!({"doi": "10.1101/2024.01.01.999999", "version": "3", "title": "latest"}),
        json!({"doi": "10.1101/2024.01.01.999999", "version": "2", "title": "middle"}),
    ];
    assert_eq!(latest_index(&versions), 1);
    let record = preprint_record(
        &json!({
            "doi": "10.1101/2024.01.01.999999",
            "version": "2",
            "title": "Synthetic oscillator",
            "published": "NA",
            "funder": "NA",
            "server": "medrxiv"
        }),
        "medrxiv",
    )
    .unwrap();
    assert_eq!(
        record["url"],
        "https://www.medrxiv.org/content/10.1101/2024.01.01.999999v2"
    );
    assert_eq!(
        record["pdf_url"],
        "https://www.medrxiv.org/content/10.1101/2024.01.01.999999v2.full.pdf"
    );
    assert_eq!(
        record["doi_url"],
        "https://doi.org/10.1101/2024.01.01.999999"
    );
    assert!(record["published_doi"].is_null());
    assert!(record["funding"].is_null());
}

fn test_bio(base: &str) -> NativeBio {
    NativeBio::test_client(
        &[("BIORXIV_API".into(), base.trim_end_matches('/').into())],
        Http(reqwest::Client::builder().no_proxy().build().unwrap()),
    )
    .unwrap()
}

async fn mock(
    status: StatusCode,
    body: String,
) -> (
    NativeBio,
    Arc<StdMutex<Vec<String>>>,
    tokio::task::JoinHandle<()>,
) {
    let captured = Arc::new(StdMutex::new(Vec::new()));
    let request_log = captured.clone();
    let app = Router::new().fallback(move |uri: Uri| {
        let request_log = request_log.clone();
        let body = body.clone();
        async move {
            request_log.lock().unwrap().push(uri.to_string());
            (status, [("retry-after", "60")], body).into_response()
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (test_bio(&endpoint), captured, task)
}

fn paper(index: usize) -> Value {
    json!({
        "doi": format!("10.1101/2024.01.01.{index:06}"),
        "title": format!("Synthetic preprint {index}"),
        "authors": "Doe, A",
        "date": "2024-01-15",
        "category": "systems biology",
        "version": "1",
        "type": "new results",
        "license": "cc_by",
        "abstract": if index == 0 {
            format!("{}UNIQUE_TAIL", "A".repeat(ABSTRACT_PREVIEW))
        } else {
            "A short synthetic abstract.".into()
        },
        "server": "biorxiv",
        "published": "NA",
        "funder": "NA"
    })
}

#[tokio::test]
async fn search_preprints_uses_shipped_dispatch_and_content_api_paths() {
    let captured = Arc::new(StdMutex::new(Vec::new()));
    let request_log = captured.clone();
    let app = Router::new().fallback(move |uri: Uri| {
        let request_log = request_log.clone();
        async move {
            request_log.lock().unwrap().push(uri.to_string());
            let path = uri.path();
            let cursor = path
                .rsplit('/')
                .nth(1)
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            let page: Vec<_> = (cursor..35).take(30).map(paper).collect();
            axum::Json(json!({
                "messages": [{"status": "ok", "total": "35", "count": page.len(), "cursor": cursor}],
                "collection": page
            }))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let bio = test_bio(&endpoint);
    let result = bio
        .call(
            "search_preprints",
            &json!({
                "date_from": "2024-01-01",
                "date_to": "2024-01-31",
                "category": "systems biology",
                "limit": 10
            }),
        )
        .await
        .unwrap();
    let first = bio
        .call("search_preprints", &json!({"recent_days": 30, "limit": 1}))
        .await
        .unwrap();
    let recent = bio
        .call(
            "search_preprints",
            &json!({"recent_count": 50, "cursor": 0, "limit": 1}),
        )
        .await
        .unwrap();
    let default = bio
        .call("search_preprints", &json!({"limit": 1}))
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["source"], "bioRxiv Content API");
    assert_eq!(result["source_url"], SOURCE_URL);
    assert_eq!(result["total"], 35);
    assert_eq!(result["returned"], 10);
    assert_eq!(result["has_more"], true);
    assert_eq!(result["next_cursor"], 10);
    assert_eq!(result["page_size"], 30);
    assert_eq!(
        result["records"][0]["url"],
        "https://www.biorxiv.org/content/10.1101/2024.01.01.000000v1"
    );
    assert_eq!(result["records"][0]["abstract_truncated"], true);
    assert!(!result["records"][0]["abstract_preview"]
        .as_str()
        .unwrap()
        .contains("UNIQUE_TAIL"));
    let log = captured.lock().unwrap();
    assert!(log.iter().any(|uri| {
        uri.contains("/details/biorxiv/2024-01-01/2024-01-31/0/json")
            && uri.contains("category=systems_biology")
    }));
    assert!(log
        .iter()
        .any(|uri| uri.contains("/details/biorxiv/30d/0/json")));
    assert!(log
        .iter()
        .any(|uri| uri.contains("/details/biorxiv/50/0/json")));
    assert!(log
        .iter()
        .any(|uri| uri.contains("/details/biorxiv/60d/0/json")));
    assert_eq!(first["interval"], "30d");
    assert_eq!(recent["interval"], "50");
    assert_eq!(default["interval"], "60d");
}

#[tokio::test]
async fn search_walks_a_second_details_page_without_inventing_totals() {
    let app = Router::new().fallback(|uri: Uri| async move {
        let cursor = uri
            .path()
            .rsplit('/')
            .nth(1)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let page: Vec<_> = (cursor..35).take(30).map(paper).collect();
        axum::Json(json!({
            "messages": [{"status": "ok", "total": 35, "count": page.len()}],
            "collection": page
        }))
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let bio = test_bio(&endpoint);
    let result = bio
        .call(
            "search_preprints",
            &json!({
                "date_from": "2024-01-01",
                "date_to": "2024-01-31",
                "limit": 50
            }),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["returned"], 35);
    assert_eq!(result["has_more"], false);
    assert!(result["next_cursor"].is_null());
}

#[tokio::test]
async fn get_preprint_reports_versions_urls_and_missing_dois() {
    let found = json!({
        "messages": [{"status": "ok", "count": 2}],
        "collection": [
            {
                "doi": "10.1101/2024.01.01.999999",
                "title": "Synthetic oscillator v1",
                "authors": "Doe, A",
                "author_corresponding": "A Doe",
                "author_corresponding_institution": "Example Institute",
                "date": "2024-01-01",
                "version": "1",
                "type": "new results",
                "category": "systems biology",
                "license": "cc_by",
                "abstract": "Version one.",
                "jatsxml": "https://www.biorxiv.org/content/10.1101/2024.01.01.999999v1.source.xml",
                "funder": "NA",
                "published": "NA",
                "server": "biorxiv"
            },
            {
                "doi": "10.1101/2024.01.01.999999",
                "title": "Synthetic oscillator v2",
                "authors": "Doe, A; Roe, B",
                "date": "2024-02-01",
                "version": "2",
                "category": "systems biology",
                "abstract": "Version two.",
                "published": "10.0000/syn.0001",
                "server": "biorxiv"
            }
        ]
    });
    let (bio, captured, server) = mock(StatusCode::OK, found.to_string()).await;
    let result = bio
        .call(
            "get_preprint",
            &json!({"doi": "https://doi.org/10.1101/2024.01.01.999999"}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["found"], true);
    assert_eq!(result["n_versions"], 2);
    assert_eq!(result["preprint"]["version"], "2");
    assert_eq!(result["preprint"]["published_doi"], "10.0000/syn.0001");
    assert_eq!(
        result["preprint"]["url"],
        "https://www.biorxiv.org/content/10.1101/2024.01.01.999999v2"
    );
    assert!(
        captured.lock().unwrap()[0].contains("/details/biorxiv/10.1101/2024.01.01.999999/na/json")
    );
    assert!(result["missing_dois"].as_array().unwrap().is_empty());

    let missing = json!({"messages": [{"status": "no posts found"}], "collection": []});
    let (bio, _, server) = mock(StatusCode::OK, missing.to_string()).await;
    let result = bio
        .call(
            "get_preprint",
            &json!({"doi": "10.1101/2024.01.01.000000", "server": "medrxiv"}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["found"], false);
    assert_eq!(result["missing_dois"], json!(["10.1101/2024.01.01.000000"]));
    assert!(result["preprint"].is_null());
}

#[tokio::test]
async fn published_and_funder_routes_encode_documented_paths() {
    let captured = Arc::new(StdMutex::new(Vec::new()));
    let request_log = captured.clone();
    let app = Router::new().fallback(move |uri: Uri| {
        let request_log = request_log.clone();
        async move {
            request_log.lock().unwrap().push(uri.to_string());
            let path = uri.path();
            if path.starts_with("/pubs/") || path.starts_with("/publisher/") {
                axum::Json(json!({
                    "messages": [{"status": "ok", "total": 1}],
                    "collection": [{
                        "biorxiv_doi": "10.1101/2024.01.01.999999",
                        "published_doi": "10.0000/syn.0001",
                        "published_journal": "Synthetic Biology Letters",
                        "preprint_platform": "biorxiv",
                        "preprint_title": "Synthetic oscillator",
                        "preprint_authors": "Doe, A",
                        "preprint_category": "systems biology",
                        "preprint_date": "2024-01-01",
                        "published_date": "2024-06-01",
                        "preprint_abstract": "A synthetic abstract."
                    }]
                }))
            } else {
                axum::Json(json!({
                    "messages": [{"status": "ok", "total": 1}],
                    "collection": [paper(1)]
                }))
            }
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let bio = test_bio(&endpoint);
    let published = bio
        .call(
            "search_published_preprints",
            &json!({
                "date_from": "2024-01-01",
                "date_to": "2024-12-31",
                "limit": 5
            }),
        )
        .await
        .unwrap();
    let summary = bio
        .call(
            "search_published_preprints",
            &json!({
                "publisher": "10.0000",
                "recent_days": 14,
                "include_details": false,
                "limit": 5
            }),
        )
        .await
        .unwrap();
    let funder = bio
        .call(
            "search_by_funder",
            &json!({
                "funder_ror_id": "https://ror.org/021nxhr62",
                "date_from": "2025-04-10",
                "date_to": "2025-05-10",
                "category": "cancer biology",
                "limit": 5
            }),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(
        published["records"][0]["preprint_doi"],
        "10.1101/2024.01.01.999999"
    );
    assert_eq!(
        published["records"][0]["published_url"],
        "https://doi.org/10.0000/syn.0001"
    );
    assert_eq!(
        published["records"][0]["preprint_url"],
        "https://www.biorxiv.org/content/10.1101/2024.01.01.999999"
    );
    assert_eq!(published["records"][0]["preprint_authors"], "Doe, A");
    assert!(summary["records"][0].get("preprint_authors").is_none());
    assert_eq!(summary["publisher"], "10.0000");
    assert_eq!(funder["funder_ror_id"], "021nxhr62");
    assert_eq!(funder["records"][0]["doi"], "10.1101/2024.01.01.000001");
    let log = captured.lock().unwrap();
    assert!(log
        .iter()
        .any(|uri| uri.contains("/pubs/biorxiv/2024-01-01/2024-12-31/0/json")));
    assert!(log
        .iter()
        .any(|uri| uri.contains("/publisher/10.0000/14d/0") && !uri.contains("/json")));
    assert!(log.iter().any(|uri| {
        uri.contains("/funder/biorxiv/2025-04-10/2025-05-10/021nxhr62/0/json")
            && uri.contains("category=cancer_biology")
    }));
}

#[tokio::test]
async fn statistics_use_documented_sum_and_usage_paths() {
    let captured = Arc::new(StdMutex::new(Vec::new()));
    let request_log = captured.clone();
    let app = Router::new().fallback(move |uri: Uri| {
        let request_log = request_log.clone();
        async move {
            request_log.lock().unwrap().push(uri.to_string());
            if uri.path().starts_with("/sum/") {
                axum::Json(json!({
                    "messages": [{"status": "ok"}],
                    "bioRxiv content statistics": [{
                        "month": "2013-11",
                        "new_papers": "2",
                        "new_papers_cumulative": "2",
                        "revised_papers": "1",
                        "preprint_date": null,
                        "revised_papers_cumulative": "1"
                    }]
                }))
            } else {
                axum::Json(json!({
                    "messages": [{"status": "ok"}],
                    "bioRxiv usage statistics": [{
                        "year": "2014",
                        "abstract_views": "10",
                        "full_text_views": "5",
                        "pdf_downloads": "2",
                        "abstract_cumulative": "10",
                        "full_text_cumulative": "5",
                        "pdf_cumulative": "2"
                    }]
                }))
            }
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let bio = test_bio(&endpoint);
    let content = bio
        .call("get_content_statistics", &json!({"interval": "monthly"}))
        .await
        .unwrap();
    let usage = bio
        .call(
            "get_usage_statistics",
            &json!({"interval": "yearly", "server": "medrxiv"}),
        )
        .await
        .unwrap();
    let categories = bio.call("get_categories", &json!({})).await.unwrap();
    server.abort();
    assert_eq!(content["records"][0]["month"], "2013-11");
    assert_eq!(content["records"][0]["new_papers"], 2);
    assert!(content["records"][0]["preprint_date"].is_null());
    assert_eq!(usage["server"], "medrxiv");
    assert_eq!(usage["records"][0]["year"], 2014);
    assert_eq!(usage["records"][0]["pdf_downloads"], 2);
    assert_eq!(categories["returned"], 27);
    let log = captured.lock().unwrap();
    assert!(log.iter().any(|uri| uri.contains("/sum/m/json")));
    assert!(log.iter().any(|uri| uri.contains("/usage/y/medrxiv/json")));
}

#[tokio::test]
async fn empty_search_is_distinct_from_malformed_or_rejected_responses() {
    let empty = json!({"messages": [{"status": "no posts found"}], "collection": []});
    let (bio, _, server) = mock(StatusCode::OK, empty.to_string()).await;
    let result = bio
        .call(
            "search_preprints",
            &json!({"date_from": "2024-01-01", "date_to": "2024-01-02"}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["returned"], 0);
    assert_eq!(result["total"], 0);
    assert_eq!(result["has_more"], false);

    for body in [
        json!({"messages": [{"status": "error", "note": "synthetic-key"}]}).to_string(),
        json!({"messages": [{"status": "ok"}], "collection": "oops"}).to_string(),
        json!({"collection": []}).to_string(),
        "not-json".into(),
    ] {
        let (bio, _, server) = mock(StatusCode::OK, body).await;
        let error = bio
            .call("search_preprints", &json!({"recent_days": 1}))
            .await
            .unwrap_err()
            .to_string();
        server.abort();
        assert!(!error.contains("synthetic-key"), "{error}");
    }
}

#[tokio::test]
async fn rejects_upstream_errors_and_oversized_responses_without_echoing_secrets() {
    for (status, body, expected) in [
        (
            StatusCode::TOO_MANY_REQUESTS,
            "synthetic-key".into(),
            "HTTP 429",
        ),
        (
            StatusCode::OK,
            " ".repeat(MAX_RESPONSE + 1),
            "exceeded 4 MiB",
        ),
    ] {
        let (bio, _, server) = mock(status, body).await;
        let error = bio
            .call("search_preprints", &json!({"recent_days": 1}))
            .await
            .unwrap_err()
            .to_string();
        server.abort();
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("synthetic-key"), "{error}");
        assert!(!error.contains("BIORXIV_API"), "{error}");
    }
}

#[tokio::test]
async fn funder_inception_and_unknown_tools_fail_closed() {
    let bio = NativeBio::new(&[]).unwrap();
    let error = bio
        .call(
            "search_by_funder",
            &json!({
                "funder_ror_id": "021nxhr62",
                "date_from": "2024-01-01",
                "date_to": "2024-02-01"
            }),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("2025-04-10"), "{error}");
    let error = crate::biorxiv::call(&bio, "not_a_tool", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown native biological tool"));
}
