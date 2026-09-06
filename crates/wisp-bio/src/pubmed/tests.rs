use super::*;
use crate::http::{Http, MAX_RESPONSE};
use axum::{
    extract::{Path, Query},
    http::{StatusCode, Uri},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

#[test]
fn validates_queries_and_upstream_retrieval_boundary() {
    for args in [
        json!({"query": " "}),
        json!({"query": "x", "max_results": 0}),
        json!({"query": "x", "max_results": 201}),
        json!({"query": "x", "retstart": 9999, "max_results": 2}),
        json!({"query": "x", "date_from": "2024"}),
        json!({"query": "x", "date_from": "2023/02/29", "date_to": "2024/01/01"}),
        json!({"query": "x", "sort": "invalid"}),
    ] {
        let search: Search = serde_json::from_value(args).unwrap();
        assert!(search.params().is_err());
    }
    assert!(serde_json::from_value::<Search>(json!({"query": "x", "api_key": "secret"})).is_err());
    let search: Search = serde_json::from_value(json!({
        "query": "gene[Title] & study", "retstart": 9999, "max_results": 1,
        "sort": "author", "date_from": "2024/02/29", "date_to": "2024/03/01"
    }))
    .unwrap();
    let params = search.params().unwrap();
    assert!(params.contains(&("sort".into(), "Author".into())));
    assert!(params.contains(&("term".into(), "gene[Title] & study".into())));
    let result = search_result(
        &json!({"esearchresult": {
            "count": "12000", "idlist": ["123"]
        }}),
        &search,
    )
    .unwrap();
    assert_eq!(result["total"], 12000);
    assert_eq!(result["has_more"], true);
    assert_eq!(result["next_retstart"], Value::Null);
}

#[test]
fn distinguishes_empty_results_from_malformed_or_rejected_responses() {
    let search: Search = serde_json::from_value(json!({"query": "fictional query"})).unwrap();
    let result = search_result(
        &json!({"esearchresult": {
            "count": "0", "idlist": []
        }}),
        &search,
    )
    .unwrap();
    assert_eq!(result["returned"], 0);
    assert_eq!(result["has_more"], false);
    for raw in [
        json!({}),
        json!({"esearchresult": {"count": "unknown", "idlist": []}}),
        json!({"esearchresult": {"count": "0", "idlist": ["123"]}}),
        json!({"esearchresult": {"count": "3", "idlist": []}}),
        json!({"esearchresult": {"count": "0", "idlist": [], "errorlist": {"fieldsnotfound": ["oops"]}}}),
    ] {
        assert!(search_result(&raw, &search).is_err());
    }
}

#[test]
fn summaries_preserve_requested_order_duplicates_and_missing_ids() {
    let ids = vec!["123".into(), "456".into(), "123".into()];
    let result = summary_result(
        &json!({"result": {
        "uids": ["123"], "123": {"uid": "123", "title": "Synthetic citation", "articleids": [{"idtype": "doi", "value": "10.example/synthetic"}]}
    }}),
        &ids,
    )
    .unwrap();
    assert_eq!(result["returned"], 2);
    assert_eq!(result["missing_pmids"], json!(["456"]));
    assert_eq!(result["records"][0], result["records"][1]);
    assert_eq!(
        result["records"][0]["url"],
        "https://pubmed.ncbi.nlm.nih.gov/123/"
    );
    assert!(summary_result(&json!({"result": {}}), &ids).is_err());
    assert!(summary_result(
        &json!({"result": {"uids": ["123"], "123": {"uid": "456"}}}),
        &ids
    )
    .is_err());
    for ids in [
        vec![],
        vec!["123&api_key=oops".into()],
        vec!["0".into()],
        vec!["1".into(); 201],
    ] {
        assert!(validate_pmids(&ids).is_err());
    }
}

fn test_client(base: &str) -> PubMed {
    let mut client = PubMed::new(&[
        ("NCBI_API_KEY".into(), "synthetic-key&value".into()),
        ("NCBI_EMAIL".into(), "operator@example.test".into()),
    ])
    .unwrap();
    client.ncbi = base.to_string();
    client.idconv = format!("{base}idconv");
    client.europepmc = base.to_string();
    client.http = Http(reqwest::Client::builder().no_proxy().build().unwrap());
    client
}

async fn mock(
    status: StatusCode,
    body: String,
) -> (PubMed, Arc<StdMutex<String>>, tokio::task::JoinHandle<()>) {
    let captured = Arc::new(StdMutex::new(String::new()));
    let request_body = captured.clone();
    let app = Router::new().route(
        "/esearch.fcgi",
        post(move |body_in: String| {
            *request_body.lock().unwrap() = body_in;
            let body = body.clone();
            async move { (status, [("retry-after", "60")], body).into_response() }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (test_client(&endpoint), captured, task)
}

#[tokio::test]
async fn http_contract_encodes_queries_and_credentials_without_python() {
    let (client, captured, server) = mock(
        StatusCode::OK,
        json!({
            "esearchresult": {"count": "3", "idlist": ["123"], "querytranslation": "synthetic"}
        })
        .to_string(),
    )
    .await;
    let result = client
        .call(
            "search_articles",
            &json!({
                "query": "gene[Title] & study", "max_results": 1
            }),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["next_retstart"], 1);
    let body = captured.lock().unwrap();
    assert!(body.contains("term=gene%5BTitle%5D+%26+study"));
    assert!(body.contains("api_key=synthetic-key%26value"));
    assert!(body.contains("email=operator%40example.test"));
    assert!(body.contains("tool=wisp-science"));
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
        let (client, _, server) = mock(status, body).await;
        let error = client
            .call("search_articles", &json!({"query": "synthetic"}))
            .await
            .unwrap_err()
            .to_string();
        server.abort();
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("synthetic-key"));
    }
}

#[test]
fn catalog_uses_deferred_read_only_tools() {
    let expected = [
        "search_articles",
        "get_article_metadata",
        "convert_article_ids",
        "find_related_articles",
        "lookup_article_by_citation",
        "get_full_text_article",
        "get_copyright_status",
    ];
    let pubmed: Vec<_> = crate::catalog()
        .into_iter()
        .filter(|(domain, _)| *domain == "pubmed")
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        pubmed,
        expected
            .iter()
            .map(|name| ("pubmed", (*name).to_string()))
            .collect::<Vec<_>>()
    );
    let tools = crate::tools(Arc::new(crate::NativeBio::new(&[]).unwrap()));
    let mut names = std::collections::HashSet::new();
    for tool in &tools {
        assert!(tool.defer_schema());
        assert!(tool.read_only());
        assert!(names.insert(tool.name().to_string()));
        assert_eq!(tool.name(), tool.schema().function.name);
    }
    assert_eq!(names.len(), crate::catalog().len());
    let pubmed_only =
        crate::tools_for_package(Arc::new(crate::NativeBio::new(&[]).unwrap()), "mcp_pubmed");
    assert_eq!(pubmed_only.len(), 7);
    assert!(crate::package_selects("mcp_bio", "pubmed"));
    assert!(crate::package_selects("mcp_pubmed", "pubmed"));
    assert!(crate::selected_by_package("mcp_chembl"));
}

#[test]
fn abstracts_handle_inline_markup_entities_sections_and_book_records() {
    let xml = br#"<?xml version="1.0"?><PubmedArticleSet>
      <PubmedArticle><MedlineCitation><PMID>123</PMID><Article><Abstract>
        <AbstractText>One <i>synthetic</i> study &amp; &#945;.</AbstractText>
        <AbstractText>Second section.</AbstractText>
      </Abstract></Article></MedlineCitation><PubmedData><ReferenceList><Reference><ArticleIdList><PMID>999</PMID></ArticleIdList></Reference></ReferenceList></PubmedData></PubmedArticle>
      <PubmedBookArticle><BookDocument><PMID>456</PMID><Abstract><AbstractText><![CDATA[A < B]]></AbstractText></Abstract></BookDocument></PubmedBookArticle>
      <PubmedArticle><MedlineCitation><PMID>789</PMID><Article /></MedlineCitation></PubmedArticle>
    </PubmedArticleSet>"#;
    let records = parse_abstracts(xml).unwrap();
    assert_eq!(records["123"], "One synthetic study & α.\nSecond section.");
    assert_eq!(records["456"], "A < B");
    assert_eq!(records["789"], "");
    assert!(!records.contains_key("999"));
    for xml in [
        b"<eFetchResult><ERROR>failure</ERROR></eFetchResult>".as_slice(),
        b"<PubmedArticleSet><PubmedArticle>",
        b"<PubmedArticleSet><PubmedArticle /></wrong>",
        b"<PubmedArticleSet/><OtherRoot/>",
        b"<PubmedArticleSet/>unexpected text",
    ] {
        assert!(
            parse_abstracts(xml).is_err(),
            "{}",
            String::from_utf8_lossy(xml)
        );
    }
    assert!(parse_abstracts(b"<PubmedArticleSet />").unwrap().is_empty());
}

#[tokio::test]
async fn metadata_combines_citations_and_abstracts_without_a_live_upstream() {
    let app = Router::new()
        .route("/esummary.fcgi", post(|| async { axum::Json(json!({"result": {
            "uids": ["123"], "123": {"uid": "123", "title": "Invented study"}
        }})) }))
        .route("/efetch.fcgi", post(|| async {
            "<PubmedArticleSet><PubmedArticle><MedlineCitation><PMID>123</PMID><Article><Abstract><AbstractText>Invented abstract.</AbstractText></Abstract></Article></MedlineCitation></PubmedArticle></PubmedArticleSet>"
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/", listener.local_addr().unwrap());
    let client = test_client(&endpoint);
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let result = client
        .call("get_article_metadata", &json!({"pmids": ["123", "456"]}))
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["records"][0]["summary"]["title"], "Invented study");
    assert_eq!(result["records"][0]["abstract"], "Invented abstract.");
    assert_eq!(result["missing_pmids"], json!(["456"]));
    assert_eq!(result["metadata_level"], "citation_and_abstract");
}

#[test]
fn convert_encodes_same_type_batches_and_reports_missing_embargoed_and_unconverted() {
    assert!(ids::parse_args(json!({"ids": ["1", "PMC2"], "id_type": "pmid"})).is_err());
    assert!(ids::parse_args(json!({"ids": ["10.example/synthetic"], "id_type": "pmid"})).is_err());
    assert!(
        ids::parse_args(json!({"ids": (1..=201).map(|n| n.to_string()).collect::<Vec<_>>()}))
            .is_err()
    );
    let (id_type, ids) =
        ids::parse_args(json!({"ids": ["3531190", "PMC555"], "id_type": "pmcid"})).unwrap();
    assert_eq!(id_type, "pmcid");
    assert_eq!(ids, vec!["PMC3531190", "PMC555"]);
    let result = ids::convert_result(
        &json!({"status": "ok", "records": [
            {"requested-id": "1", "pmid": 1, "pmcid": "PMC555", "doi": "10.example/synthetic", "live": false, "release-date": "2027-03-04"},
            {"requested-id": "3", "pmid": "3", "status": "error", "errmsg": "invalid article id"},
            {"requested-id": "4", "pmid": "4"}
        ]}),
        &["1".into(), "2".into(), "3".into(), "4".into()],
        "pmid",
    )
    .unwrap();
    assert_eq!(result["source"], "NCBI PMC ID Converter");
    assert_eq!(result["source_url"], ids::SOURCE_URL);
    assert!(!result["source_url"]
        .as_str()
        .unwrap()
        .contains("oa-service"));
    assert!(!result["source_url"].as_str().unwrap().contains("oa.fcgi"));
    assert_eq!(result["records"][0]["pmcid"], "PMC555");
    assert_eq!(result["records"][0]["live"], false);
    assert_eq!(result["records"][0]["release_date"], "2027-03-04");
    assert_eq!(result["missing_ids"], json!(["2"]));
    assert_eq!(result["unconverted_ids"][0]["requested_id"], "3");
    assert_eq!(result["unconverted_ids"][1]["requested_id"], "4");
    assert!(result["unconverted_ids"][1]["reason"]
        .as_str()
        .unwrap()
        .contains("not in PubMed Central"));
}

#[test]
fn elink_preserves_upstream_ranking_and_bounds_the_page() {
    assert!(
        related::parse_spec(json!({"pmids": ["1"], "link_type": "pubmed_pubmed_citedin"})).is_err()
    );
    let result = related::parse_related(
        &json!({"linksets": [{"dbfrom": "pubmed", "ids": [10], "linksetdbs": [{
            "dbto": "pubmed", "linkname": "pubmed_pubmed", "links": [
                {"id": "301", "score": "9"},
                {"id": 302, "score": 8},
                {"id": "303", "score": "1"}
            ]
        }]}]}),
        json!({"pmids": ["10"], "max_results": 2}),
    )
    .unwrap();
    assert_eq!(result["ranking"], "upstream_elink_order");
    assert_eq!(result["has_more"], true);
    assert_eq!(
        result["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|record| record["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["301", "302"]
    );
    assert_eq!(result["records"][0]["score"], "9");
    assert!(related::parse_related(
        &json!({"linksets": [{"ERROR": "invalid uid"}]}),
        json!({"pmids": ["10"]})
    )
    .is_err());
}

#[test]
fn citmatch_encodes_documented_pipes_and_splits_matched_from_unmatched() {
    let encoded = citations::encode_args(json!({"citations": [
        {"journal": "proc natl acad sci u s a", "year": 1991, "volume": "88", "first_page": "3248", "author": "mann bj", "key": "Art1"},
        {"journal": "science", "year": "1987", "first_page": "182"}
    ]}))
    .unwrap();
    assert_eq!(
        encoded,
        "proc+natl+acad+sci+u+s+a|1991|88|3248|mann+bj|Art1|\rscience|1987||182||c2|"
    );
    let result = citations::parse_body(
        "proc+natl+acad+sci+u+s+a|1991|88|3248|mann+bj|Art1|20142531\nscience|1987||182||c2|NOT_FOUND\n",
        json!({"citations": [
            {"journal": "proc natl acad sci u s a", "year": 1991, "volume": "88", "first_page": "3248", "author": "mann bj", "key": "Art1"},
            {"journal": "science", "year": "1987", "first_page": "182"}
        ]}),
    )
    .unwrap();
    assert_eq!(result["matched"][0]["pmid"], "20142531");
    assert_eq!(result["unmatched"][0]["key"], "c2");
    assert!(citations::parse_body(
        "<eLinkResult/>",
        json!({"citations": [{"journal": "Nature"}]})
    )
    .is_err());
}

#[tokio::test]
async fn convert_dispatches_through_the_id_converter_without_python_or_oa_service() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = captured.clone();
    let app = Router::new().route(
        "/idconv",
        get(move |uri: Uri| {
            seen.lock().unwrap().push(uri.to_string());
            async move {
                axum::Json(json!({
                    "status": "ok",
                    "records": [
                        {"requested-id": "1", "pmid": "1", "pmcid": "PMC555", "live": true},
                        {"requested-id": "2", "pmid": "2"}
                    ]
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/", listener.local_addr().unwrap());
    let client = test_client(&endpoint);
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let result = client
        .call(
            "convert_article_ids",
            &json!({"ids": ["1", "2", "3"], "id_type": "pmid"}),
        )
        .await
        .unwrap();
    server.abort();
    let urls = captured.lock().unwrap().join(" ");
    assert!(urls.contains("ids=1%2C2"));
    assert!(urls.contains("idtype=pmid"));
    assert!(urls.contains("format=json"));
    assert!(urls.contains("email=operator%40example.test"));
    assert!(!urls.contains("api_key"));
    assert!(!urls.contains("oa.fcgi"));
    assert!(!urls.contains("oa-service"));
    assert_eq!(result["records"][0]["pmcid"], "PMC555");
    assert_eq!(result["unconverted_ids"][0]["requested_id"], "2");
    assert_eq!(result["missing_ids"], json!(["3"]));
    assert!(!result.to_string().contains("synthetic-key"));
}

#[tokio::test]
async fn related_articles_dispatch_preserves_elink_order() {
    let captured = Arc::new(StdMutex::new(String::new()));
    let body = captured.clone();
    let app = Router::new().route(
        "/elink.fcgi",
        post(move |request: String| {
            *body.lock().unwrap() = request;
            async move {
                axum::Json(
                    json!({"linksets": [{"dbfrom": "pubmed", "ids": ["10"], "linksetdbs": [{
                        "dbto": "pubmed", "linkname": "pubmed_pubmed",
                        "links": [{"id": "301", "score": "20"}, {"id": "302", "score": "10"}]
                    }]}]}),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = test_client(&format!("http://{}/", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let result = client
        .call(
            "find_related_articles",
            &json!({"pmids": ["10"], "link_type": "pubmed_pubmed", "max_results": 1}),
        )
        .await
        .unwrap();
    server.abort();
    let request = captured.lock().unwrap().clone();
    assert!(request.contains("linkname=pubmed_pubmed"));
    assert!(request.contains("cmd=neighbor_score"));
    assert!(request.contains("id=10"));
    assert_eq!(result["records"][0]["id"], "301");
    assert_eq!(result["returned"], 1);
    assert_eq!(result["has_more"], true);
}

#[tokio::test]
async fn citation_lookup_dispatch_reports_matched_and_unmatched() {
    let app = Router::new().route(
        "/ecitmatch.cgi",
        post(|body: String| async move {
            assert!(body.contains("bdata=nature%7C2020%7C580%7C123%7Csmith%7Ck1%7C"));
            assert!(body.contains("retmode=xml"));
            assert!(!body.contains("rettype="));
            "nature|2020|580|123|smith|k1|999\nlancet|2021||||k2|NOT_FOUND\n"
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = test_client(&format!("http://{}/", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let result = client
        .call(
            "lookup_article_by_citation",
            &json!({"citations": [
                {"journal": "nature", "year": 2020, "volume": "580", "first_page": "123", "author": "smith", "key": "k1"},
                {"journal": "lancet", "year": 2021, "key": "k2"}
            ]}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["matched"][0]["pmid"], "999");
    assert_eq!(result["unmatched"][0]["key"], "k2");
    assert_eq!(result["source"], "NCBI ECitMatch");
}

#[tokio::test]
async fn full_text_dispatch_distinguishes_not_found_not_oa_and_xml_unavailable() {
    const JATS: &str = r#"<article><front><article-meta>
        <article-id pub-id-type="pmcid">PMC555</article-id>
        <title-group><article-title>Invented OA article</article-title></title-group>
        <abstract><p>Invented abstract.</p></abstract>
      </article-meta></front>
      <body><sec><title>Intro</title><p>Invented body text.</p></sec></body></article>"#;
    let app = Router::new()
        .route(
            "/search",
            get(|Query(params): Query<HashMap<String, String>>| async move {
                assert_eq!(params.get("resultType").map(String::as_str), Some("core"));
                assert_eq!(params.get("format").map(String::as_str), Some("json"));
                axum::Json(json!({
                    "hitCount": 3,
                    "resultList": {"result": [
                        {"pmcid": "PMC555", "pmid": "1", "isOpenAccess": "Y", "inEPMC": "Y", "title": "Invented OA article"},
                        {"pmcid": "PMC556", "pmid": "2", "isOpenAccess": "N", "inEPMC": "Y"},
                        {"pmcid": "PMC557", "pmid": "3", "isOpenAccess": "Y", "inEPMC": "Y"}
                    ]}
                }))
            }),
        )
        .route(
            "/{id}/fullTextXML",
            get(|Path(id): Path<String>| async move {
                match id.as_str() {
                    "PMC555" => JATS.to_string().into_response(),
                    "PMC557" => StatusCode::NOT_FOUND.into_response(),
                    other => panic!("unexpected PMCID {other}"),
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = test_client(&format!("http://{}/", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let result = client
        .call(
            "get_full_text_article",
            &json!({"pmc_ids": ["PMC555", "PMC556", "PMC557", "PMC558"]}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["records"][0]["pmcid"], "PMC555");
    assert!(result["records"][0]["full_text"]
        .as_str()
        .unwrap()
        .contains("Invented body text"));
    assert_eq!(result["not_open_access"][0]["pmcid"], "PMC556");
    assert_eq!(result["xml_unavailable"][0]["pmcid"], "PMC557");
    assert_eq!(result["not_found"], json!(["PMC558"]));
    assert_ne!(result["records"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn copyright_dispatch_keeps_oa_flag_distinct_from_reuse_and_skips_retired_oa_service() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = captured.clone();
    let app = Router::new()
        .route(
            "/search",
            get({
                let seen = seen.clone();
                move |uri: Uri| {
                    seen.lock().unwrap().push(uri.to_string());
                    async move {
                        axum::Json(json!({
                            "hitCount": 2,
                            "resultList": {"result": [
                                {"source": "MED", "pmid": "1", "pmcid": "PMC555", "isOpenAccess": "Y", "inEPMC": "Y", "license": "cc by", "doi": "10.example/synthetic"},
                                {"source": "MED", "pmid": "2", "pmcid": "PMC556", "isOpenAccess": "Y", "inEPMC": "N"}
                            ]}
                        }))
                    }
                }
            }),
        )
        .route(
            "/idconv",
            get({
                let seen = seen.clone();
                move |uri: Uri| {
                    seen.lock().unwrap().push(uri.to_string());
                    async move {
                        axum::Json(json!({
                            "status": "ok",
                            "records": [
                                {"requested-id": "1", "pmid": "1", "pmcid": "PMC555", "live": false, "release-date": "2027-01-01"},
                                {"requested-id": "2", "pmid": "2", "pmcid": "PMC556", "live": true}
                            ]
                        }))
                    }
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = test_client(&format!("http://{}/", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let result = client
        .call("get_copyright_status", &json!({"pmids": ["1", "2", "3"]}))
        .await
        .unwrap();
    server.abort();
    let urls = captured.lock().unwrap().join(" ");
    assert!(urls.contains("/search"));
    assert!(urls.contains("SRC%3AMED") || urls.contains("SRC:MED"));
    assert!(urls.contains("/idconv"));
    assert!(!urls.contains("oa.fcgi"));
    assert!(!urls.contains("oa-service"));
    assert!(!urls.contains("pmc/utils/oa"));
    assert!(result["contract_note"]
        .as_str()
        .unwrap()
        .contains("not a reuse grant"));
    assert_eq!(result["records"][0]["is_open_access"], true);
    assert_eq!(result["records"][0]["reuse_permission"], "license_stated");
    assert_eq!(result["records"][0]["embargo"]["live"], false);
    assert_eq!(result["records"][1]["is_open_access"], true);
    assert_eq!(result["records"][1]["full_text_accessible"], false);
    assert_eq!(result["records"][1]["reuse_permission"], "unknown");
    assert!(result["records"][1].get("reuse_granted").is_none());
    assert_eq!(result["missing_pmids"], json!(["3"]));
    assert!(!result.to_string().contains("synthetic-key"));
}

#[test]
fn copyright_medline_query_uses_documented_unique_ext_id_form() {
    assert_eq!(
        europepmc::medline_query_for_test(&["526631".into(), "2".into()]),
        "(EXT_ID:526631 AND SRC:MED) OR (EXT_ID:2 AND SRC:MED)"
    );
}

#[tokio::test]
async fn copyright_scopes_pmids_to_medline_so_colliding_sources_do_not_consume_the_page() {
    let captured = Arc::new(StdMutex::new(String::new()));
    let seen = captured.clone();
    let app = Router::new()
        .route(
            "/search",
            get({
                let seen = seen.clone();
                move |Query(params): Query<HashMap<String, String>>| {
                    let query = params.get("query").cloned().unwrap_or_default();
                    *seen.lock().unwrap() = query.clone();
                    let page_size = params.get("pageSize").cloned().unwrap_or_default();
                    async move {
                        // A bare EXT_ID matches MEDLINE and PMC. With pageSize equal
                        // to the PMID count, the PMC hit would occupy the only slot.
                        let scoped = query.contains("SRC:MED") && query.contains("EXT_ID:526631");
                        let results = if scoped {
                            json!([{
                                "source": "MED",
                                "id": "526631",
                                "pmid": "526631",
                                "pmcid": "PMC999",
                                "isOpenAccess": "Y",
                                "inEPMC": "Y",
                                "license": "cc by"
                            }])
                        } else {
                            json!([{
                                "source": "PMC",
                                "id": "526631",
                                "pmcid": "PMC526631",
                                "isOpenAccess": "N"
                            }])
                        };
                        (
                            [("x-page-size", page_size)],
                            axum::Json(json!({
                                "hitCount": 2,
                                "resultList": {"result": results}
                            })),
                        )
                    }
                }
            }),
        )
        .route(
            "/idconv",
            get(|| async {
                axum::Json(json!({
                    "status": "ok",
                    "records": [{
                        "requested-id": "526631",
                        "pmid": "526631",
                        "pmcid": "PMC999",
                        "live": true
                    }]
                }))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = test_client(&format!("http://{}/", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let result = client
        .call("get_copyright_status", &json!({"pmids": ["526631"]}))
        .await
        .unwrap();
    server.abort();
    let query = captured.lock().unwrap().clone();
    assert!(
        query.contains("(EXT_ID:526631 AND SRC:MED)"),
        "copyright lookup must use the unique MEDLINE form, got {query}"
    );
    assert_eq!(result["missing_pmids"], json!([]));
    assert_eq!(result["records"][0]["pmid"], "526631");
    assert_eq!(result["records"][0]["license"]["name"], "cc by");
    assert_eq!(result["returned"], 1);
}

#[tokio::test]
async fn convert_rejects_http_429_and_malformed_json_without_echoing_secrets() {
    for (status, body, expected) in [
        (
            StatusCode::TOO_MANY_REQUESTS,
            "synthetic-key".into(),
            "HTTP 429",
        ),
        (StatusCode::OK, "{not-json".into(), "invalid JSON"),
        (
            StatusCode::OK,
            json!({"status": "error", "error": "synthetic-key"}).to_string(),
            "rejected",
        ),
    ] {
        let app = Router::new().route(
            "/idconv",
            get({
                let body = body.clone();
                move || {
                    let body = body.clone();
                    async move { (status, [("retry-after", "60")], body).into_response() }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = test_client(&format!("http://{}/", listener.local_addr().unwrap()));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let error = client
            .call("convert_article_ids", &json!({"ids": ["1"]}))
            .await
            .unwrap_err()
            .to_string();
        server.abort();
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("synthetic-key"));
    }
}

#[test]
fn full_text_jats_rejects_malformed_or_mismatched_documents() {
    assert!(records::full_text("<not-article/>", "PMC555").is_err());
    assert!(records::full_text(
        "<article><front><article-meta><article-id pub-id-type=\"pmcid\">PMC1</article-id></article-meta></front></article>",
        "PMC555",
    )
    .is_err());
    let parsed = records::full_text(
        "<article><front><article-meta><article-id pub-id-type=\"pmcid\">PMC555</article-id><title-group><article-title>Invented</article-title></title-group></article-meta></front><body><p>Body.</p></body></article>",
        "PMC555",
    )
    .unwrap();
    assert_eq!(parsed["title"], "Invented");
    assert!(parsed["full_text"].as_str().unwrap().contains("Body"));
}

#[test]
fn default_endpoints_do_not_include_the_retired_oa_service() {
    let client = PubMed::new(&[]).unwrap();
    for url in [&client.ncbi, &client.idconv, &client.europepmc] {
        assert!(!url.contains("oa.fcgi"), "{url}");
        assert!(!url.contains("oa-service"), "{url}");
        assert!(!url.contains("pmc/utils/oa"), "{url}");
    }
    assert_eq!(client.idconv, IDCONV);
    assert_eq!(client.europepmc, EUROPE_PMC_REST);
}
