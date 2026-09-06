use super::*;
use crate::http::{Http, MAX_RESPONSE};
use axum::{
    extract::Query,
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
        &[("BIOMART_BASE_URL".into(), base.trim_end_matches('/').into())],
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

fn registry_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<MartRegistry>
  <MartURLLocation name="ENSEMBL_MART_ENSEMBL" displayName="Ensembl Genes" database="ensembl_mart_synthetic" host="www.ensembl.org" path="/biomart/martservice" serverVirtualSchema="default" visible="1" default="1" />
  <MartURLLocation name="ENSEMBL_MART_SNP" displayName="Ensembl Variation" database="snp_mart_synthetic" host="www.ensembl.org" visible="1" default="" serverVirtualSchema="default" />
</MartRegistry>
"#
}

fn datasets_tsv() -> &'static str {
    "TableSet\thsapiens_gene_ensembl\tHuman genes (GRCh38.p14)\t1\tGRCh38.p14\t\t\tdefault\t2026-01-01\n\
     TableSet\tmmusculus_gene_ensembl\tMouse genes (GRCm39)\t1\tGRCm39\t\t\tdefault\t2026-01-01\n\
     \n\
     IgnoreMe\tnot_a_dataset\n"
}

fn attributes_tsv() -> &'static str {
    "ensembl_gene_id\tGene stable ID\tStable ID of the gene\tfeature_page\thtml,txt,csv,tsv,xls\tgene__main\tstable_id\n\
     hgnc_symbol\tGene name\tHGNC symbol\tfeature_page\thtml,txt,csv,tsv,xls\tgene__main\tdisplay_label\n\
     chromosome_name\tChromosome/scaffold name\tChromosome name\tfeature_page\thtml,txt,csv,tsv,xls\tgene__main\tname\n\
     mmusculus_homolog_ensembl_gene\tMouse gene stable ID\tMouse ortholog\thomologs\thtml,txt,csv,tsv,xls\thomolog_mmusculus\tstable_id\n"
}

fn filters_tsv() -> &'static str {
    "ensembl_gene_id\tGene stable ID(s)\t[]\t\tid_list\tid_list\t=\tgene__main\tstable_id\n\
     hgnc_symbol\tHGNC symbol(s)\t[]\t\tid_list\tid_list\t=\tgene__main\tdisplay_label\n\
     chromosome_name\tChromosome/scaffold name\t[1,2,X,Y,MT]\tChromosome name\tfeature_page\tlist\t=\tgene__main\tname\n\
     with_uniprotswissprot\tWith UniProtKB/Swiss-Prot ID\t\t\tfeature_page\tboolean\tonly\tgene__main\t\n"
}

fn query_ok(body: &str) -> String {
    format!("{body}\n[success]\n")
}

#[test]
fn catalog_registers_eight_read_only_biomart_tools() {
    let names: Vec<_> = catalog()
        .into_iter()
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        names,
        vec![
            ("biomart", "list_marts".into()),
            ("biomart", "list_datasets".into()),
            ("biomart", "list_common_attributes".into()),
            ("biomart", "list_all_attributes".into()),
            ("biomart", "list_filters".into()),
            ("biomart", "get_data".into()),
            ("biomart", "get_translation".into()),
            ("biomart", "batch_translate".into()),
        ]
    );
    assert!(crate::contains_tool("list_marts"));
    assert_eq!(crate::domain_for_tool("get_data"), Some("biomart"));
    assert!(crate::package_selects("mcp_biomart", "biomart"));
    assert!(crate::selected_by_package("mcp_biomart"));
}

#[test]
fn rejects_unbounded_or_malformed_arguments() {
    for args in [
        json!({"mart": ""}),
        json!({"mart": "ENSEMBL MART"}),
        json!({"mart": "ENSEMBL_MART_ENSEMBL\"/>"}),
        json!({"mart": "ENSEMBL_MART_ENSEMBL", "max_results": 0}),
        json!({"mart": "ENSEMBL_MART_ENSEMBL", "max_results": 501}),
        json!({"mart": "ENSEMBL_MART_ENSEMBL", "api_key": "secret"}),
    ] {
        match serde_json::from_value::<ListDatasets>(args.clone()) {
            Ok(parsed) => assert!(
                require_ident(&parsed.mart, "mart").is_err()
                    || bound_page(parsed.max_results).is_err(),
                "{args}"
            ),
            Err(_) => {}
        }
    }
    assert!(require_ident("hsapiens_gene_ensembl", "dataset").is_ok());
    assert!(require_ident("1bad", "dataset").is_err());
    assert!(require_value("HLA-A", "target").is_ok());
    assert!(require_value("TP53,BRCA1", "target").is_err());
    assert!(require_attributes(&[]).is_err());
    assert!(require_attributes(&["ensembl_gene_id".into(), "ensembl_gene_id".into()]).is_err());
    assert!(require_filters(&BTreeMap::new()).is_err());
    assert!(serde_json::from_value::<GetData>(json!({
        "mart": "ENSEMBL_MART_ENSEMBL",
        "dataset": "hsapiens_gene_ensembl",
        "attributes": ["ensembl_gene_id"],
        "filters": {"hgnc_symbol": "SYNTH1"},
        "api_key": "secret"
    }))
    .is_err());
    assert!(bound_page(0).is_err());
    assert!(ensembl_id_url("ENSG00000000001").is_some());
    assert!(ensembl_id_url("TP53").is_none());
    assert!(ensembl_id_url("../ENSG00000000001").is_none());
}

#[test]
fn parses_registry_datasets_attributes_and_filters() {
    let marts = parse_registry(registry_xml()).unwrap();
    assert_eq!(marts.len(), 2);
    assert_eq!(marts[0]["name"], "ENSEMBL_MART_ENSEMBL");
    assert_eq!(marts[0]["visible"], true);
    assert_eq!(marts[0]["default"], true);
    assert_eq!(marts[1]["default"], false);
    assert!(parse_registry("<html><body>nope</body></html>").is_err());
    assert!(parse_registry("<MartRegistry/>").is_err());

    let datasets = parse_datasets(datasets_tsv(), "ENSEMBL_MART_ENSEMBL").unwrap();
    assert_eq!(datasets.len(), 2);
    assert_eq!(datasets[0]["name"], "hsapiens_gene_ensembl");
    assert_eq!(datasets[0]["assembly"], "GRCh38.p14");
    assert!(parse_datasets("not a table\n", "ENSEMBL_MART_ENSEMBL").is_err());

    let attrs = parse_attributes(attributes_tsv(), "hsapiens_gene_ensembl").unwrap();
    assert_eq!(attrs.len(), 4);
    assert_eq!(
        select_page(&attrs, None, true).as_deref(),
        Some("feature_page")
    );
    assert_eq!(select_page(&attrs, None, false), None);
    let common: Vec<_> = attrs
        .iter()
        .filter(|a| a.page == "feature_page")
        .map(|a| a.name.as_str())
        .collect();
    assert_eq!(
        common,
        vec!["ensembl_gene_id", "hgnc_symbol", "chromosome_name"]
    );
    assert!(!common.contains(&"mmusculus_homolog_ensembl_gene"));

    let filters = parse_filters(filters_tsv(), "hsapiens_gene_ensembl").unwrap();
    assert_eq!(filters[2]["n_options"], 5);
    assert_eq!(filters[3]["type"], "boolean");
    assert!(parse_filters("short\trow\n", "hsapiens_gene_ensembl").is_err());
}

#[test]
fn query_xml_encodes_filters_attributes_and_completion_stamp() {
    let xml = build_query_xml(
        "hsapiens_gene_ensembl",
        &["ensembl_gene_id".into(), "hgnc_symbol".into()],
        &[
            Filter::List("hgnc_symbol".into(), vec!["SYNTH1".into(), "SYNTH2".into()]),
            Filter::Include("with_uniprotswissprot".into()),
            Filter::Value("chromosome_name".into(), "X".into()),
        ],
    )
    .unwrap();
    assert!(xml.contains("completionStamp=\"1\""), "{xml}");
    assert!(xml.contains("formatter=\"TSV\""), "{xml}");
    assert!(xml.contains("header=\"0\""), "{xml}");
    assert!(
        xml.contains("Dataset name=\"hsapiens_gene_ensembl\""),
        "{xml}"
    );
    assert!(
        xml.contains("Filter name=\"hgnc_symbol\" value=\"SYNTH1,SYNTH2\""),
        "{xml}"
    );
    assert!(
        xml.contains("Filter name=\"with_uniprotswissprot\" excluded=\"0\""),
        "{xml}"
    );
    assert!(xml.contains("Attribute name=\"ensembl_gene_id\""), "{xml}");
    assert!(xml.contains("<!DOCTYPE Query>"), "{xml}");
    let escaped = build_query_xml(
        "hsapiens_gene_ensembl",
        &["ensembl_gene_id".into()],
        &[Filter::Value("hgnc_symbol".into(), "A&B".into())],
    )
    .unwrap();
    assert!(escaped.contains("value=\"A&amp;B\""), "{escaped}");
    assert!(complete_tsv("ENSG1\tSYNTH1\n[success]\n").is_ok());
    assert!(complete_tsv("ENSG1\tSYNTH1\n").is_err());
    assert_eq!(
        parse_tsv_rows("ENSG00000000001\tSYNTH1\nENSG00000000002\tSYNTH2", 2)
            .unwrap()
            .len(),
        2
    );
    assert!(parse_tsv_rows("only-one-column", 2).is_err());
}

#[test]
fn encodes_boolean_and_list_filters_from_json() {
    let mut filters = BTreeMap::new();
    filters.insert("with_uniprotswissprot".into(), json!(true));
    filters.insert("biotype".into(), json!("protein_coding"));
    filters.insert("hgnc_symbol".into(), json!(["SYNTH1", "SYNTH2"]));
    filters.insert("start".into(), json!(1000));
    let encoded = require_filters(&filters).unwrap();
    assert!(encoded.contains(&Filter::Include("with_uniprotswissprot".into())));
    assert!(encoded.contains(&Filter::Value("biotype".into(), "protein_coding".into())));
    assert!(encoded.contains(&Filter::List(
        "hgnc_symbol".into(),
        vec!["SYNTH1".into(), "SYNTH2".into()]
    )));
    assert!(encoded.contains(&Filter::Value("start".into(), "1000".into())));
    let mut excluded = BTreeMap::new();
    excluded.insert("with_uniprotswissprot".into(), json!("excluded"));
    assert_eq!(
        require_filters(&excluded).unwrap(),
        vec![Filter::Exclude("with_uniprotswissprot".into())]
    );
}

#[tokio::test]
async fn list_tools_dispatch_through_native_bio_call() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = captured.clone();
    let app = Router::new().route(
        "/",
        get(move |Query(params): Query<HashMap<String, String>>| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(format!("{params:?}"));
                match params.get("type").map(String::as_str) {
                    Some("registry") => registry_xml().to_string().into_response(),
                    Some("datasets") => datasets_tsv().to_string().into_response(),
                    Some("attributes") => attributes_tsv().to_string().into_response(),
                    Some("filters") => filters_tsv().to_string().into_response(),
                    _ => (StatusCode::BAD_REQUEST, "unexpected").into_response(),
                }
            }
        }),
    );
    let (bio, server) = serve(app).await;
    let marts = bio
        .call("list_marts", &json!({"max_results": 1}))
        .await
        .unwrap();
    let datasets = bio
        .call(
            "list_datasets",
            &json!({"mart": "ENSEMBL_MART_ENSEMBL", "max_results": 10}),
        )
        .await
        .unwrap();
    let common = bio
        .call(
            "list_common_attributes",
            &json!({
                "mart": "ENSEMBL_MART_ENSEMBL",
                "dataset": "hsapiens_gene_ensembl"
            }),
        )
        .await
        .unwrap();
    let all = bio
        .call(
            "list_all_attributes",
            &json!({
                "mart": "ENSEMBL_MART_ENSEMBL",
                "dataset": "hsapiens_gene_ensembl",
                "max_results": 2
            }),
        )
        .await
        .unwrap();
    let filters = bio
        .call(
            "list_filters",
            &json!({
                "mart": "ENSEMBL_MART_ENSEMBL",
                "dataset": "hsapiens_gene_ensembl"
            }),
        )
        .await
        .unwrap();
    server.abort();
    let traffic = captured.lock().unwrap().join("\n");
    assert!(traffic.contains("registry"), "{traffic}");
    assert!(traffic.contains("datasets"), "{traffic}");
    assert!(traffic.contains("attributes"), "{traffic}");
    assert!(traffic.contains("filters"), "{traffic}");
    assert_eq!(marts["source"], "Ensembl BioMart");
    assert_eq!(marts["source_url"], MARTSERVICE);
    assert_eq!(marts["martview_url"], MARTVIEW);
    assert_eq!(marts["returned"], 1);
    assert_eq!(marts["truncated"], true);
    assert_eq!(marts["total_available"], 2);
    assert_eq!(datasets["datasets"][0]["name"], "hsapiens_gene_ensembl");
    assert_eq!(common["query"]["page"], "feature_page");
    assert_eq!(common["returned"], 3);
    assert!(common["attributes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|attr| attr["page"] == "feature_page"));
    assert_eq!(all["total_available"], 4);
    assert_eq!(all["returned"], 2);
    assert_eq!(all["truncated"], true);
    assert_eq!(filters["filters"][2]["n_options"], 5);
}

#[tokio::test]
async fn get_data_posts_query_xml_and_bounds_rows_with_source_urls() {
    let captured = Arc::new(StdMutex::new(String::new()));
    let body = captured.clone();
    let app = Router::new().route(
        "/",
        post(move |incoming: String| {
            let body = body.clone();
            async move {
                *body.lock().unwrap() = incoming;
                query_ok(
                    "ENSG00000000001\tSYNTH1\nENSG00000000002\tSYNTH2\nENSG00000000003\tSYNTH3",
                )
                .into_response()
            }
        }),
    );
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "get_data",
            &json!({
                "mart": "ENSEMBL_MART_ENSEMBL",
                "dataset": "hsapiens_gene_ensembl",
                "attributes": ["ensembl_gene_id", "hgnc_symbol"],
                "filters": {"hgnc_symbol": ["SYNTH1", "SYNTH2", "SYNTH3"]},
                "max_results": 2
            }),
        )
        .await
        .unwrap();
    server.abort();
    let form = captured.lock().unwrap().clone();
    assert!(form.contains("query="), "{form}");
    assert!(
        form.contains("completionStamp%3D%221%22") || form.contains("completionStamp=\"1\""),
        "{form}"
    );
    assert!(form.contains("hsapiens_gene_ensembl"), "{form}");
    assert!(form.contains("hgnc_symbol"), "{form}");
    assert_eq!(result["total_available"], 3);
    assert_eq!(result["returned"], 2);
    assert_eq!(result["truncated"], true);
    assert_eq!(result["records"][0]["ensembl_gene_id"], "ENSG00000000001");
    assert_eq!(
        result["records"][0]["url"],
        "https://www.ensembl.org/id/ENSG00000000001"
    );
    assert_eq!(result["source_url"], MARTSERVICE);
}

#[tokio::test]
async fn translation_uses_id_list_filter_and_reports_missing() {
    let captured = Arc::new(StdMutex::new(String::new()));
    let body = captured.clone();
    let app = Router::new().route(
        "/",
        post(move |incoming: String| {
            let body = body.clone();
            async move {
                *body.lock().unwrap() = incoming;
                query_ok(
                    "SYNTH1\tENSG00000000001\nSYNTH1\tENSG00000000001\nSYNTH2\tENSG00000000002",
                )
                .into_response()
            }
        }),
    );
    let (bio, server) = serve(app).await;
    let one = bio
        .call(
            "get_translation",
            &json!({
                "mart": "ENSEMBL_MART_ENSEMBL",
                "dataset": "hsapiens_gene_ensembl",
                "from_attr": "hgnc_symbol",
                "to_attr": "ensembl_gene_id",
                "target": "SYNTH1"
            }),
        )
        .await
        .unwrap();
    let batch = bio
        .call(
            "batch_translate",
            &json!({
                "mart": "ENSEMBL_MART_ENSEMBL",
                "dataset": "hsapiens_gene_ensembl",
                "from_attr": "hgnc_symbol",
                "to_attr": "ensembl_gene_id",
                "targets": ["SYNTH1", "SYNTH2", "MISSING1"]
            }),
        )
        .await
        .unwrap();
    server.abort();
    let form = captured.lock().unwrap().clone();
    assert!(form.contains("hgnc_symbol"), "{form}");
    assert!(
        form.contains("SYNTH1") || form.contains("SYNTH1%2C"),
        "{form}"
    );
    assert!(!form.contains("type=configuration"), "{form}");
    assert_eq!(one["found"], true);
    assert_eq!(one["value"], "ENSG00000000001");
    assert_eq!(one["url"], "https://www.ensembl.org/id/ENSG00000000001");
    assert_eq!(batch["found_count"], 2);
    assert_eq!(batch["not_found"], json!(["MISSING1"]));
    assert_eq!(batch["translations"]["SYNTH1"], "ENSG00000000001");
    assert_eq!(batch["records"][2]["found"], false);
}

#[tokio::test]
async fn empty_complete_query_is_not_an_error() {
    let app = Router::new().route("/", post(|| async { query_ok("").into_response() }));
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "get_data",
            &json!({
                "mart": "ENSEMBL_MART_ENSEMBL",
                "dataset": "hsapiens_gene_ensembl",
                "attributes": ["ensembl_gene_id"],
                "filters": {"hgnc_symbol": "NOSUCH"}
            }),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["returned"], 0);
    assert_eq!(result["total_available"], 0);
    assert_eq!(result["truncated"], false);
}

#[tokio::test]
async fn rejects_rate_limits_html_query_errors_and_truncated_tsv() {
    for (status, body, expected) in [
        (
            StatusCode::TOO_MANY_REQUESTS,
            "secret-token".to_string(),
            "HTTP 429",
        ),
        (
            StatusCode::OK,
            "<!doctype html><html><body>maintenance</body></html>".to_string(),
            "HTML page",
        ),
        (
            StatusCode::OK,
            "Query ERROR: caught BioMart::Exception::Usage: Attribute nope NOT FOUND\n".to_string(),
            "Query ERROR",
        ),
        (
            StatusCode::OK,
            "ENSG00000000001\tSYNTH1\n".to_string(),
            "truncated",
        ),
    ] {
        let app = Router::new().route(
            "/",
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
            .call(
                "get_data",
                &json!({
                    "mart": "ENSEMBL_MART_ENSEMBL",
                    "dataset": "hsapiens_gene_ensembl",
                    "attributes": ["ensembl_gene_id", "hgnc_symbol"],
                    "filters": {"hgnc_symbol": "SYNTH1"}
                }),
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
    let app = Router::new().route(
        "/",
        post(|| async { (StatusCode::OK, " ".repeat(MAX_RESPONSE + 1)).into_response() }),
    );
    let (bio, server) = serve(app).await;
    let error = bio
        .call(
            "get_data",
            &json!({
                "mart": "ENSEMBL_MART_ENSEMBL",
                "dataset": "hsapiens_gene_ensembl",
                "attributes": ["ensembl_gene_id"],
                "filters": {"chromosome_name": "1"}
            }),
        )
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("exceeded 4 MiB"), "{error}");
}

#[tokio::test]
async fn metadata_html_and_empty_registry_are_errors() {
    let app = Router::new().route(
        "/",
        get(|Query(params): Query<HashMap<String, String>>| async move {
            match params.get("type").map(String::as_str) {
                Some("registry") => "<MartRegistry></MartRegistry>".into_response(),
                _ => "<html>outage</html>".into_response(),
            }
        }),
    );
    let (bio, server) = serve(app).await;
    let registry = bio
        .call("list_marts", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    let datasets = bio
        .call("list_datasets", &json!({"mart": "ENSEMBL_MART_ENSEMBL"}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(registry.contains("no marts"), "{registry}");
    assert!(datasets.contains("HTML page"), "{datasets}");
}

#[tokio::test]
async fn unknown_tool_name_is_rejected() {
    let (bio, server) = serve(Router::new()).await;
    let error = call(&bio, "biomart_not_a_tool", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("unknown native biological tool"));
}

#[tokio::test]
async fn get_data_rejects_unfiltered_queries_before_http() {
    let (bio, server) = serve(Router::new()).await;
    let error = bio
        .call(
            "get_data",
            &json!({
                "mart": "ENSEMBL_MART_ENSEMBL",
                "dataset": "hsapiens_gene_ensembl",
                "attributes": ["ensembl_gene_id"],
                "filters": {}
            }),
        )
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("at least one filter"), "{error}");
}
