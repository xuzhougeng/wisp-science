use super::*;
use crate::http::{Http, MAX_RESPONSE};
use crate::NativeBio;
use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, Uri},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

#[test]
fn catalog_registers_literature_tools() {
    let expected = [
        "openalex_search_works",
        "openalex_get_work",
        "openalex_get_author",
        "openalex_search_authors",
        "openalex_citations",
        "openalex_references",
        "openalex_venue_info",
        "arxiv_search",
        "arxiv_get_papers",
    ];
    let tools: Vec<_> = crate::catalog()
        .into_iter()
        .filter(|(domain, _)| *domain == "literature")
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        tools,
        expected
            .iter()
            .map(|name| ("literature", (*name).to_string()))
            .collect::<Vec<_>>()
    );
    assert!(crate::package_selects("mcp_literature", "literature"));
    assert!(crate::package_selects("mcp_bio", "literature"));
}

#[tokio::test]
async fn rejects_unknown_fields_bounds_and_missing_identifiers() {
    let bio = NativeBio::new(&[]).unwrap();
    for (name, args) in [
        (
            "openalex_search_works",
            json!({"query": "x", "api_key": "secret"}),
        ),
        (
            "openalex_search_works",
            json!({"query": "x", "max_records": 0}),
        ),
        (
            "openalex_search_works",
            json!({"query": "x", "max_records": 201}),
        ),
        (
            "openalex_search_works",
            json!({"query": "x", "sort": "invalid"}),
        ),
        (
            "openalex_search_works",
            json!({"year_from": 2024, "year_to": 2020}),
        ),
        ("openalex_get_work", json!({"work_id": "not-an-id"})),
        (
            "openalex_get_work",
            json!({"work_id": "10.example/synthetic,extra"}),
        ),
        ("openalex_get_author", json!({"author_id": "W111"})),
        (
            "openalex_venue_info",
            json!({"venue": "https://openalex.org/W111"}),
        ),
        ("arxiv_search", json!({})),
        ("arxiv_search", json!({"query": "x", "max_results": 101})),
        ("arxiv_search", json!({"query": "x", "sort_by": "date"})),
        ("arxiv_search", json!({"date_from": "2024-02-30"})),
        ("arxiv_get_papers", json!({"arxiv_ids": []})),
        (
            "arxiv_get_papers",
            json!({"arxiv_ids": ["x"], "api_key": "secret"}),
        ),
    ] {
        let error = bio.call(name, &args).await.unwrap_err().to_string();
        assert!(
            !error.contains("secret"),
            "{name} {args} leaked a secret: {error}"
        );
        assert!(
            error.contains("invalid")
                || error.contains("must")
                || error.contains("unrecognized")
                || error.contains("provide")
                || error.contains("not a")
                || error.contains("unsupported")
                || error.contains("dates"),
            "{name} {args} -> {error}"
        );
    }
}

#[test]
fn reconstructs_open_abstracts_and_omits_restricted_licenses() {
    let index = json!({"Invented": [0], "abstract": [2], "text": [1]});
    assert_eq!(
        openalex::reconstruct_abstract(&index).as_deref(),
        Some("Invented text abstract")
    );
    assert_eq!(openalex::reconstruct_abstract(&json!({})), None);
    let open = json!({
        "id": "https://openalex.org/W111",
        "title": "Invented CRISPR study",
        "primary_location": {"license": "https://openalex.org/licenses/cc-by"},
        "abstract_inverted_index": {"Invented": [0], "abstract": [1]}
    });
    let record = openalex::lean_work(&open, true);
    assert_eq!(record["abstract"], "Invented abstract");
    assert_eq!(record["abstract_license"], "cc-by");
    assert_eq!(record["url"], "https://openalex.org/W111");
    assert_eq!(record["provider"], "OpenAlex");
    let closed = json!({
        "id": "https://openalex.org/W222",
        "title": "Invented restricted study",
        "primary_location": {"license": "cc-by-nc"},
        "abstract_inverted_index": {"Secret": [0]}
    });
    let record = openalex::lean_work(&closed, true);
    assert_eq!(record["abstract"], Value::Null);
    assert_eq!(record["abstract_license"], "cc-by-nc");
    assert!(record["abstract_policy"]
        .as_str()
        .unwrap()
        .contains("cc-by-nc"));
    let undeclared = openalex::lean_work(&json!({"id": "https://openalex.org/W333"}), true);
    assert_eq!(undeclared["abstract"], Value::Null);
    assert!(undeclared["abstract_policy"]
        .as_str()
        .unwrap()
        .contains("not declared"));
}

#[test]
fn normalizes_openalex_and_arxiv_identifiers() {
    assert_eq!(
        openalex::normalize_work_id("https://openalex.org/W2981137429").unwrap(),
        "W2981137429"
    );
    assert_eq!(
        openalex::normalize_work_id("doi:10.example/synthetic").unwrap(),
        "doi:10.example/synthetic"
    );
    assert_eq!(
        openalex::normalize_author_id("https://orcid.org/0000-0002-9943-7557").unwrap(),
        "orcid:0000-0002-9943-7557"
    );
    assert_eq!(
        openalex::normalize_source_id("1087-0156").unwrap(),
        "issn:1087-0156"
    );
    assert!(openalex::normalize_work_id("https://openalex.org/A111").is_err());
    assert!(openalex::normalize_work_id("10.example/a|b").is_err());
    assert_eq!(
        arxiv::normalize_arxiv_id("https://arxiv.org/pdf/2103.14030v2.pdf").unwrap(),
        "2103.14030v2"
    );
    assert_eq!(
        arxiv::normalize_arxiv_id("arXiv:q-bio/0601001").unwrap(),
        "q-bio/0601001"
    );
}

#[test]
fn arxiv_feed_detects_error_envelope_and_parses_entries() {
    let parsed = arxiv::parse_feed(
        r#"<?xml version="1.0" encoding="utf-8"?>
        <feed xmlns="http://www.w3.org/2005/Atom" xmlns:opensearch="http://a9.com/-/spec/opensearch/1.1/" xmlns:arxiv="http://arxiv.org/schemas/atom">
          <opensearch:totalResults>2</opensearch:totalResults>
          <opensearch:startIndex>0</opensearch:startIndex>
          <entry>
            <id>http://arxiv.org/abs/2103.99999v1</id>
            <title>Invented preprint</title>
            <summary>Invented abstract.</summary>
            <published>2021-03-01T00:00:00Z</published>
            <author><name>A. Scientist</name></author>
            <link rel="related" title="pdf" href="https://arxiv.org/pdf/2103.99999v1" type="application/pdf"/>
            <arxiv:primary_category term="q-bio.GN"/>
            <category term="q-bio.GN"/>
            <arxiv:doi>10.example/synthetic</arxiv:doi>
          </entry>
        </feed>"#,
    )
    .unwrap();
    assert_eq!(parsed.total, 2);
    assert_eq!(parsed.records[0]["arxiv_id"], "2103.99999");
    assert_eq!(parsed.records[0]["version"], 1);
    assert_eq!(parsed.records[0]["doi"], "10.example/synthetic");
    assert_eq!(parsed.records[0]["primary_category"], "q-bio.GN");
    let error = arxiv::parse_feed(
        r#"<?xml version="1.0" encoding="utf-8"?>
        <feed xmlns="http://www.w3.org/2005/Atom">
          <entry>
            <id>http://arxiv.org/api/errors#incorrect_id_format_for_bad</id>
            <title>Error</title>
            <summary>incorrect id format for bad</summary>
          </entry>
        </feed>"#,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("incorrect id format"));
    assert!(
        arxiv::parse_feed("<feed><!DOCTYPE foo><entry/></feed>").is_err()
            || arxiv::parse_feed(
                "<?xml version='1.0'?><!DOCTYPE feed><feed xmlns='http://www.w3.org/2005/Atom'/>"
            )
            .is_err()
    );
}

fn test_bio() -> NativeBio {
    NativeBio::test_client(
        &[
            ("OPENALEX_API_KEY".into(), "synthetic-key&value".into()),
            ("NCBI_EMAIL".into(), "operator@example.test".into()),
        ],
        Http(reqwest::Client::builder().no_proxy().build().unwrap()),
    )
    .unwrap()
}

async fn dispatch(bio: &NativeBio, base: &str, name: &str, args: Value) -> Result<Value, String> {
    TEST_ENDPOINTS
        .scope((base.to_string(), format!("{base}/api/query")), async {
            bio.call(name, &args).await
        })
        .await
        .map_err(|error| error.to_string())
}

#[derive(Clone)]
struct Fake {
    captured: Arc<StdMutex<Vec<String>>>,
}

async fn literature_server() -> (
    NativeBio,
    String,
    Arc<StdMutex<Vec<String>>>,
    tokio::task::JoinHandle<()>,
) {
    let captured = Arc::new(StdMutex::new(Vec::new()));
    let app = Router::new()
        .route("/works", get(works))
        .route("/works/{id}", get(one_work))
        .route("/authors", get(authors))
        .route("/authors/{id}", get(one_author))
        .route("/sources", get(sources))
        .route("/sources/{id}", get(one_source))
        .route("/api/query", get(arxiv))
        .with_state(Fake {
            captured: captured.clone(),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (test_bio(), endpoint, captured, task)
}

fn work_w111() -> Value {
    json!({
        "id": "https://openalex.org/W111",
        "doi": "https://doi.org/10.example/synthetic",
        "ids": {"pmid": "https://pubmed.ncbi.nlm.nih.gov/123"},
        "title": "Invented CRISPR study",
        "publication_year": 2024,
        "publication_date": "2024-01-15",
        "type": "article",
        "language": "en",
        "is_retracted": false,
        "authorships": [{
            "author_position": "first",
            "is_corresponding": true,
            "author": {
                "id": "https://openalex.org/A111",
                "display_name": "Invented Author",
                "orcid": "https://orcid.org/0000-0002-9943-7557"
            },
            "institutions": [{"display_name": "Invented Institute"}]
        }],
        "primary_location": {
            "license": "cc-by",
            "source": {
                "id": "https://openalex.org/S111",
                "display_name": "Invented Journal",
                "issn_l": "1234-5678",
                "type": "journal"
            }
        },
        "biblio": {"volume": "1"},
        "cited_by_count": 10,
        "fwci": 1.5,
        "referenced_works_count": 2,
        "referenced_works": ["https://openalex.org/W222", "https://openalex.org/W333"],
        "open_access": {"is_oa": true, "oa_status": "gold", "oa_url": "https://example.test/oa"},
        "best_oa_location": {"pdf_url": "https://example.test/paper.pdf", "license": "cc-by"},
        "primary_topic": {"display_name": "Invented Topic"},
        "keywords": [{"display_name": "CRISPR"}],
        "abstract_inverted_index": {"Invented": [0], "abstract": [1]},
        "counts_by_year": [{"year": 2024, "cited_by_count": 10}]
    })
}

fn work_w222() -> Value {
    json!({
        "id": "https://openalex.org/W222",
        "title": "Invented reference",
        "cited_by_count": 3,
        "type": "article"
    })
}

fn work_w999() -> Value {
    json!({
        "id": "https://openalex.org/W999",
        "title": "Invented DOI duplicate",
        "publication_year": 2024,
        "cited_by_count": 0,
        "doi": "https://doi.org/10.example/synthetic"
    })
}

fn work_w444() -> Value {
    json!({
        "id": "https://openalex.org/W444",
        "title": "Invented citing work",
        "cited_by_count": 1
    })
}

fn author_a111() -> Value {
    json!({
        "id": "https://openalex.org/A111",
        "display_name": "Invented Author",
        "orcid": "https://orcid.org/0000-0002-9943-7557",
        "works_count": 4,
        "cited_by_count": 20,
        "summary_stats": {"h_index": 3, "i10_index": 1},
        "affiliations": [{"institution": {"display_name": "Invented Institute"}, "years": [2020, 2024]}],
        "last_known_institutions": [{"display_name": "Invented Institute"}],
        "topics": [{"display_name": "Invented Topic"}],
        "counts_by_year": [{"year": 2024, "works_count": 1, "cited_by_count": 5}]
    })
}

fn source_s111() -> Value {
    json!({
        "id": "https://openalex.org/S111",
        "display_name": "Invented Journal",
        "type": "journal",
        "issn_l": "1234-5678",
        "issn": ["1234-5678"],
        "host_organization_name": "Invented Publisher",
        "country_code": "US",
        "homepage_url": "https://example.test/journal",
        "is_oa": true,
        "is_in_doaj": true,
        "is_core": true,
        "apc_usd": 0,
        "works_count": 10,
        "cited_by_count": 50,
        "summary_stats": {"h_index": 8, "2yr_mean_citedness": 2.5},
        "first_publication_year": 2000,
        "last_publication_year": 2024,
        "topics": [{"display_name": "Invented Topic"}],
        "counts_by_year": [{"year": 2024, "works_count": 2, "cited_by_count": 5}]
    })
}

fn listed(count: u64, results: Vec<Value>) -> impl IntoResponse {
    Json(json!({"meta": {"count": count}, "results": results}))
}

async fn works(
    State(fake): State<Fake>,
    uri: Uri,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    fake.captured.lock().unwrap().push(uri.to_string());
    if let Some(filter) = query.get("filter") {
        if let Some(doi) = filter.strip_prefix("doi:") {
            if doi == "10.example/synthetic" {
                return listed(2, vec![work_w111(), work_w999()]).into_response();
            }
            return listed(0, vec![]).into_response();
        }
        if filter.starts_with("cites:") {
            return listed(1, vec![work_w444()]).into_response();
        }
        if let Some(ids) = filter.strip_prefix("openalex:") {
            let rows: Vec<Value> = ids
                .split('|')
                .filter_map(|id| match id {
                    "W111" => Some(work_w111()),
                    "W222" => Some(work_w222()),
                    _ => None,
                })
                .collect();
            return listed(rows.len() as u64, rows).into_response();
        }
        if filter.starts_with("author.id:") {
            return listed(4, vec![work_w111()]).into_response();
        }
        if filter.contains("primary_location.source.id:S111")
            || filter.contains("primary_location.source.issn:1234-5678")
        {
            return listed(3, vec![work_w111()]).into_response();
        }
    }
    if query.get("search").is_some() {
        return listed(3, vec![work_w111()]).into_response();
    }
    listed(0, vec![]).into_response()
}

async fn one_work(State(fake): State<Fake>, Path(id): Path<String>, uri: Uri) -> impl IntoResponse {
    fake.captured.lock().unwrap().push(uri.to_string());
    match id.as_str() {
        "W111" => Json(work_w111()).into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn authors(
    State(fake): State<Fake>,
    uri: Uri,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    fake.captured.lock().unwrap().push(uri.to_string());
    if query.get("search").is_some() {
        listed(8, vec![author_a111()]).into_response()
    } else {
        listed(0, vec![]).into_response()
    }
}

async fn one_author(
    State(fake): State<Fake>,
    Path(id): Path<String>,
    uri: Uri,
) -> impl IntoResponse {
    fake.captured.lock().unwrap().push(uri.to_string());
    if id == "A111" || id == "orcid:0000-0002-9943-7557" {
        Json(author_a111()).into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

async fn sources(
    State(fake): State<Fake>,
    uri: Uri,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    fake.captured.lock().unwrap().push(uri.to_string());
    if query.get("search").is_some() {
        listed(2, vec![source_s111()]).into_response()
    } else {
        listed(0, vec![]).into_response()
    }
}

async fn one_source(
    State(fake): State<Fake>,
    Path(id): Path<String>,
    uri: Uri,
) -> impl IntoResponse {
    fake.captured.lock().unwrap().push(uri.to_string());
    if id == "S111" || id == "issn:1234-5678" {
        Json(source_s111()).into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

fn arxiv_entry() -> &'static str {
    r#"<entry>
      <id>http://arxiv.org/abs/2103.99999v1</id>
      <title>Invented preprint</title>
      <summary>Invented abstract.</summary>
      <published>2021-03-01T00:00:00Z</published>
      <updated>2021-03-02T00:00:00Z</updated>
      <author><name>A. Scientist</name></author>
      <link rel="alternate" href="https://arxiv.org/abs/2103.99999v1" type="text/html"/>
      <link rel="related" title="pdf" href="https://arxiv.org/pdf/2103.99999v1" type="application/pdf"/>
      <arxiv:primary_category xmlns:arxiv="http://arxiv.org/schemas/atom" term="q-bio.GN"/>
      <category term="q-bio.GN"/>
      <arxiv:doi xmlns:arxiv="http://arxiv.org/schemas/atom">10.example/synthetic</arxiv:doi>
      <arxiv:comment xmlns:arxiv="http://arxiv.org/schemas/atom">Invented comment</arxiv:comment>
    </entry>"#
}

async fn arxiv(
    State(fake): State<Fake>,
    uri: Uri,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    fake.captured.lock().unwrap().push(uri.to_string());
    if query
        .get("search_query")
        .is_some_and(|query| query.contains("trigger-error"))
    {
        return (
            StatusCode::OK,
            r#"<?xml version="1.0" encoding="utf-8"?>
            <feed xmlns="http://www.w3.org/2005/Atom">
              <entry>
                <id>http://arxiv.org/api/errors#incorrect_id_format_for_bad</id>
                <title>Error</title>
                <summary>incorrect id format for bad</summary>
              </entry>
            </feed>"#,
        )
            .into_response();
    }
    let include_entry = query
        .get("id_list")
        .is_none_or(|ids| ids.split(',').any(|id| id.contains("2103.99999")));
    let total = if query.get("id_list").is_some() {
        usize::from(include_entry)
    } else {
        4
    };
    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
        <feed xmlns="http://www.w3.org/2005/Atom" xmlns:opensearch="http://a9.com/-/spec/opensearch/1.1/" xmlns:arxiv="http://arxiv.org/schemas/atom">
          <opensearch:totalResults>{total}</opensearch:totalResults>
          <opensearch:startIndex>0</opensearch:startIndex>
          {}
        </feed>"#,
        if include_entry {
            arxiv_entry().to_string()
        } else {
            String::new()
        }
    );
    (StatusCode::OK, body).into_response()
}

#[tokio::test]
async fn search_works_encodes_credentials_without_echoing_secrets() {
    let (bio, endpoint, captured, server) = literature_server().await;
    let result = dispatch(
        &bio,
        &endpoint,
        "openalex_search_works",
        json!({
            "query": "CRISPR & study",
            "year_from": 2020,
            "year_to": 2024,
            "work_type": "article",
            "max_records": 1,
            "include_abstracts": true
        }),
    )
    .await
    .unwrap();
    server.abort();
    assert_eq!(result["provider"], "OpenAlex");
    assert_eq!(result["source_url"], "https://api.openalex.org");
    assert_eq!(result["api_total"], 3);
    assert_eq!(result["n_records_returned"], 1);
    assert_eq!(result["records_truncated"], true);
    assert_eq!(result["records"][0]["openalex_id"], "W111");
    assert_eq!(result["records"][0]["pmid"], "123");
    assert_eq!(result["records"][0]["url"], "https://openalex.org/W111");
    assert_eq!(result["records"][0]["abstract"], "Invented abstract");
    let urls = captured.lock().unwrap().join(" ");
    assert!(urls.contains("search=CRISPR"));
    assert!(urls.contains("api_key=synthetic-key%26value"));
    assert!(urls.contains("mailto=operator%40example.test"));
    assert!(
        urls.contains("publication_year%3A2020-2024")
            || urls.contains("publication_year:2020-2024")
    );
    assert!(!result.to_string().contains("synthetic-key"));
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
            json!({"error": "invalid api_key synthetic-key"}).to_string(),
            "rejected",
        ),
        (
            StatusCode::OK,
            " ".repeat(MAX_RESPONSE + 1),
            "exceeded 4 MiB",
        ),
    ] {
        let captured = Arc::new(StdMutex::new(Vec::new()));
        let body = body.clone();
        let app = Router::new().route(
            "/works",
            get({
                let captured = captured.clone();
                move |uri: Uri| {
                    captured.lock().unwrap().push(uri.to_string());
                    let body = body.clone();
                    async move { (status, [("retry-after", "60")], body).into_response() }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let error = dispatch(
            &test_bio(),
            &endpoint,
            "openalex_search_works",
            json!({"query": "synthetic"}),
        )
        .await
        .unwrap_err();
        server.abort();
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("synthetic-key"));
        assert!(captured
            .lock()
            .unwrap()
            .join(" ")
            .contains("api_key=synthetic-key%26value"));
    }
}

#[tokio::test]
async fn get_work_resolves_doi_claimants_and_missing_ids() {
    let (bio, endpoint, _, server) = literature_server().await;
    let result = dispatch(
        &bio,
        &endpoint,
        "openalex_get_work",
        json!({"work_id": "10.example/synthetic"}),
    )
    .await
    .unwrap();
    assert_eq!(result["openalex_id"], "W111");
    assert_eq!(result["abstract"], "Invented abstract");
    assert_eq!(result["referenced_works"], json!(["W222", "W333"]));
    assert_eq!(result["doi_claimants"][1]["openalex_id"], "W999");
    assert!(result["doi_resolution_note"]
        .as_str()
        .unwrap()
        .contains("W111"));
    let missing = dispatch(
        &bio,
        &endpoint,
        "openalex_get_work",
        json!({"work_id": "W404"}),
    )
    .await
    .unwrap_err();
    server.abort();
    assert!(missing.contains("HTTP 404"));
    assert!(!missing.contains("synthetic-key"));
}

#[tokio::test]
async fn citations_references_authors_and_venues_use_source_links() {
    let (bio, endpoint, captured, server) = literature_server().await;
    let citations = dispatch(
        &bio,
        &endpoint,
        "openalex_citations",
        json!({"work_id": "W111", "max_records": 1}),
    )
    .await
    .unwrap();
    assert_eq!(citations["records"][0]["openalex_id"], "W444");
    assert_eq!(citations["api_total"], 1);
    let references = dispatch(
        &bio,
        &endpoint,
        "openalex_references",
        json!({"work_id": "W111"}),
    )
    .await
    .unwrap();
    assert_eq!(references["reference_ids"], json!(["W222", "W333"]));
    assert_eq!(references["references_not_hydrated"], json!(["W333"]));
    assert_eq!(references["records"][0]["openalex_id"], "W222");
    assert_eq!(references["n_records_returned"], 1);
    let authors = dispatch(
        &bio,
        &endpoint,
        "openalex_search_authors",
        json!({"query": "Invented Author", "max_records": 1}),
    )
    .await
    .unwrap();
    assert_eq!(authors["records_truncated"], true);
    assert_eq!(authors["records"][0]["author_id"], "A111");
    let author = dispatch(
        &bio,
        &endpoint,
        "openalex_get_author",
        json!({"author_id": "A111", "works_sample": 1}),
    )
    .await
    .unwrap();
    assert_eq!(author["top_works"][0]["openalex_id"], "W111");
    assert_eq!(author["url"], "https://openalex.org/A111");
    let venue = dispatch(
        &bio,
        &endpoint,
        "openalex_venue_info",
        json!({"venue": "Invented Journal"}),
    )
    .await
    .unwrap();
    assert_eq!(venue["records"][0]["source_id"], "S111");
    let exact = dispatch(
        &bio,
        &endpoint,
        "openalex_venue_info",
        json!({"venue": "1234-5678"}),
    )
    .await
    .unwrap();
    assert_eq!(exact["source_id"], "S111");
    assert_eq!(exact["url"], "https://openalex.org/S111");
    let named = dispatch(
        &bio,
        &endpoint,
        "openalex_search_works",
        json!({"venue": "Invented Journal", "max_records": 1}),
    )
    .await
    .unwrap();
    server.abort();
    assert_eq!(named["venue_resolved"]["source_id"], "S111");
    assert_eq!(named["records"][0]["openalex_id"], "W111");
    let urls = captured.lock().unwrap().join(" ");
    assert!(urls.contains("/sources"));
    assert!(!named.to_string().contains("synthetic-key"));
}

#[tokio::test]
async fn arxiv_search_and_get_papers_report_missing_and_duplicates() {
    let (bio, endpoint, captured, server) = literature_server().await;
    let search = dispatch(
        &bio,
        &endpoint,
        "arxiv_search",
        json!({
            "query": "protein language model",
            "category": "q-bio.GN",
            "date_from": "2021-01-01",
            "date_to": "2021-12-31",
            "max_results": 1
        }),
    )
    .await
    .unwrap();
    assert_eq!(search["provider"], "arXiv");
    assert_eq!(search["n_records_returned"], 1);
    assert_eq!(search["records_truncated"], true);
    assert_eq!(search["records"][0]["arxiv_id"], "2103.99999");
    assert_eq!(
        search["records"][0]["url"],
        "http://arxiv.org/abs/2103.99999v1"
    );
    assert!(search["search_query"]
        .as_str()
        .unwrap()
        .contains("cat:q-bio.GN"));
    let papers = dispatch(
        &bio,
        &endpoint,
        "arxiv_get_papers",
        json!({
            "arxiv_ids": [
                "https://arxiv.org/abs/2103.99999",
                "2103.99999v1",
                "9999.99999",
                "not an id"
            ]
        }),
    )
    .await
    .unwrap();
    assert_eq!(papers["n_found"], 1);
    assert_eq!(papers["duplicates"][0]["requested"], "2103.99999v1");
    assert!(papers["not_found"]
        .as_array()
        .unwrap()
        .iter()
        .any(|id| id == "9999.99999"));
    assert!(papers["not_found"]
        .as_array()
        .unwrap()
        .iter()
        .any(|id| id == "not an id"));
    let error = dispatch(
        &bio,
        &endpoint,
        "arxiv_search",
        json!({"query": "trigger-error"}),
    )
    .await
    .unwrap_err();
    server.abort();
    assert!(error.contains("incorrect id format"));
    let urls = captured.lock().unwrap().join(" ");
    assert!(urls.contains("/api/query"));
    assert!(!urls.contains("api_key"));
    assert!(!papers.to_string().contains("synthetic-key"));
}
