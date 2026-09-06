use super::*;
use crate::http::{Http, MAX_RESPONSE};
use crate::NativeBio;
use axum::{
    extract::Query,
    http::{StatusCode, Uri},
    response::IntoResponse,
    routing::get,
    Router,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

fn gene_record() -> Value {
    json!({
        "gencodeId": "ENSG00000000001.1",
        "geneSymbol": "SYNTH1",
        "geneSymbolUpper": "SYNTH1",
        "gencodeVersion": "v26",
        "genomeBuild": "GRCh38/hg38",
        "geneType": "protein_coding",
        "chromosome": "chr1",
        "start": 100,
        "end": 200,
        "strand": "+",
        "tss": 100,
        "entrezGeneId": 1,
        "description": "synthetic gene",
        "geneStatus": "KNOWN",
        "dataSource": "GENCODE"
    })
}

fn paged(rows: Vec<Value>, total: u64, page: u64, pages: u64) -> Value {
    json!({
        "data": rows,
        "paging_info": {
            "numberOfPages": pages,
            "page": page,
            "maxItemsPerPage": 250,
            "totalNumberOfItems": total
        }
    })
}

fn test_bio(base: &str) -> NativeBio {
    NativeBio::test_client(
        &[("GTEX_BASE_URL".into(), base.trim_end_matches('/').into())],
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
fn catalog_registers_twelve_gtex_tools_and_skips_panglaodb() {
    let names: Vec<_> = catalog()
        .into_iter()
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        names,
        vec![
            ("expression", "gtex_calculate_eqtl".into()),
            ("expression", "gtex_dataset_info".into()),
            ("expression", "gtex_eqtl_genes".into()),
            ("expression", "gtex_expression_summary".into()),
            ("expression", "gtex_gene_expression".into()),
            ("expression", "gtex_median_expression".into()),
            ("expression", "gtex_multi_tissue_eqtls".into()),
            ("expression", "gtex_resolve_genes".into()),
            ("expression", "gtex_sample_info".into()),
            ("expression", "gtex_single_tissue_eqtls".into()),
            ("expression", "gtex_tissue_sites".into()),
            ("expression", "gtex_top_expressed_genes".into()),
        ]
    );
    assert!(crate::contains_tool("gtex_tissue_sites"));
    assert_eq!(
        crate::domain_for_tool("gtex_median_expression"),
        Some("expression")
    );
    assert!(crate::package_selects("mcp_expression", "expression"));
    assert!(crate::selected_by_package("mcp_expression"));
    assert!(!crate::contains_tool("panglaodb_marker_genes"));
    assert!(!crate::contains_tool("panglaodb_options"));
    assert!(!crate::contains_tool("panglaodb_cell_types_for_gene"));
    assert_eq!(crate::domain_for_tool("panglaodb_marker_genes"), None);
}

#[test]
fn rejects_unbounded_or_malformed_arguments() {
    assert!(serde_json::from_value::<ResolveGenes>(
        json!({"gene_ids": ["GAPDH"], "api_key": "secret"})
    )
    .is_err());
    assert!(require_ids(&[], MAX_GENE_IDS, "gene id").is_err());
    assert!(require_ids(&[" ".into()], MAX_GENE_IDS, "gene id").is_err());
    assert!(require_ids(
        &["ENSG00000000001,ENSG00000000002".into()],
        MAX_GENE_IDS,
        "gene id"
    )
    .is_err());
    assert!(require_ids(&vec!["SYNTH1".into(); 26], MAX_GENE_IDS, "gene id").is_err());
    assert!(require_token("../Liver", "tissue", TOKEN_MAX).is_err());
    assert!(require_token("Liver/cortex", "tissue", TOKEN_MAX).is_err());
    assert!(dataset_id("gtex_v7").is_err());
    assert!(dataset_id("gtex_v8").is_ok());
    assert!(dataset_id("gtex_v10").is_ok());
    assert!(data_type_id("rnaseq").is_err());
    assert_eq!(data_type_id("RNASEQ").unwrap(), "RNASEQ");
    assert!(subject_id("GTEX-14753").is_ok());
    assert!(subject_id("not-a-subject").is_err());
    assert!(subject_id("GTEX-14753&x").is_err());
    assert!(bound_page(0, MAX_RESULTS, "max_results").is_err());
    assert!(bound_page(501, MAX_RESULTS, "max_results").is_err());
    assert!(bound_page(50, MAX_TOP, "n").is_ok());
    assert!(bound_page(201, MAX_TOP, "n").is_err());
    let parsed: SingleTissueEqtls = serde_json::from_value(json!({})).unwrap();
    assert!(parsed.gencode_id.is_none() && parsed.variant_id.is_none());
}

#[test]
fn paging_and_unversioned_helpers_distinguish_empty_from_malformed() {
    let page = parse_page(&paged(
        vec![json!({"sampleId": "GTEX-AAAA-0001-SM-XXXXX"})],
        1,
        0,
        1,
    ))
    .unwrap();
    assert_eq!(page.total, 1);
    assert!(parse_page(&json!({"data": []})).is_err());
    assert!(parse_page(&json!({"data": [1], "paging_info": {
        "numberOfPages": 1, "page": 0, "totalNumberOfItems": 1
    }}))
    .is_err());
    assert!(is_unversioned_ensg("ENSG00000000001"));
    assert!(!is_unversioned_ensg("ENSG00000000001.1"));
    assert!(!is_unversioned_ensg("SYNTH1"));
    let mut payload = json!({
        "genotypes": [1, 0, 2],
        "data": [3.0, 1.0, 2.0]
    });
    sort_eqtl_pairs(&mut payload);
    assert_eq!(payload["genotypes"], json!([0, 1, 2]));
    assert_eq!(payload["data"], json!([1.0, 3.0, 2.0]));
}

#[tokio::test]
async fn tissue_sites_and_dataset_info_report_source_urls() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = captured.clone();
    let app = Router::new()
        .route(
            "/dataset/tissueSiteDetail",
            get({
                let seen = seen.clone();
                move |uri: Uri| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(uri.to_string());
                        axum::Json(paged(
                            vec![
                                json!({
                                    "tissueSiteDetailId": "Whole_Blood",
                                    "tissueSiteDetail": "Whole Blood",
                                    "tissueSite": "Blood",
                                    "eGeneCount": 3,
                                    "hasEGenes": true,
                                    "datasetId": "gtex_v8"
                                }),
                                json!({
                                    "tissueSiteDetailId": "Liver",
                                    "tissueSiteDetail": "Liver",
                                    "tissueSite": "Liver",
                                    "eGeneCount": 2,
                                    "hasEGenes": true,
                                    "datasetId": "gtex_v8"
                                }),
                            ],
                            2,
                            0,
                            1,
                        ))
                    }
                }
            }),
        )
        .route(
            "/metadata/dataset",
            get({
                let seen = seen.clone();
                move |uri: Uri| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(uri.to_string());
                        axum::Json(json!([{
                            "datasetId": "gtex_v8",
                            "displayName": "GTEx V8",
                            "gencodeVersion": "v26",
                            "genomeBuild": "GRCh38/hg38",
                            "dbSnpBuild": 151,
                            "subjectCount": 1,
                            "tissueCount": 2
                        }]))
                    }
                }
            }),
        );
    let (bio, server) = serve(app).await;
    let tissues = bio
        .call("gtex_tissue_sites", &json!({"dataset_id": "gtex_v8"}))
        .await
        .unwrap();
    let datasets = bio.call("gtex_dataset_info", &json!({})).await.unwrap();
    server.abort();
    let traffic = captured.lock().unwrap().join("\n");
    assert!(traffic.contains("/dataset/tissueSiteDetail"), "{traffic}");
    assert!(traffic.contains("datasetId=gtex_v8"), "{traffic}");
    assert!(traffic.contains("page=0"), "{traffic}");
    assert!(traffic.contains("/metadata/dataset"), "{traffic}");
    assert_eq!(tissues["source"], SOURCE);
    assert_eq!(tissues["source_url"], GTEX_API);
    assert_eq!(tissues["total"], 2);
    assert_eq!(tissues["returned"], 2);
    assert_eq!(tissues["truncated"], false);
    assert_eq!(tissues["tissue_sites"][0]["tissueSiteDetailId"], "Liver");
    assert_eq!(
        tissues["tissue_sites"][0]["url"],
        "https://gtexportal.org/home/tissue/Liver"
    );
    assert_eq!(datasets["source_url"], GTEX_API);
    assert_eq!(datasets["datasets"][0]["datasetId"], "gtex_v8");
}

#[tokio::test]
async fn remaining_tools_dispatch_through_native_bio_call() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = captured.clone();
    let capture = move |body: Value| {
        let seen = seen.clone();
        move |uri: Uri| {
            let seen = seen.clone();
            let body = body.clone();
            async move {
                seen.lock().unwrap().push(uri.to_string());
                axum::Json(body).into_response()
            }
        }
    };
    let median = json!({
        "gencodeId": "ENSG00000000001.1",
        "geneSymbol": "SYNTH1",
        "tissueSiteDetailId": "Liver",
        "median": 12.5,
        "unit": "TPM",
        "datasetId": "gtex_v8"
    });
    let app = Router::new()
        .route(
            "/reference/gene",
            get(capture(paged(vec![gene_record()], 1, 0, 1))),
        )
        .route(
            "/expression/medianGeneExpression",
            get(capture(paged(vec![median.clone()], 1, 0, 1))),
        )
        .route(
            "/expression/geneExpression",
            get(capture(paged(
                vec![json!({
                    "tissueSiteDetailId": "Liver",
                    "gencodeId": "ENSG00000000001.1",
                    "geneSymbol": "SYNTH1",
                    "unit": "TPM",
                    "data": [1.0, 2.0, 0.5]
                })],
                1,
                0,
                1,
            ))),
        )
        .route(
            "/expression/topExpressedGene",
            get(capture(paged(
                vec![json!({
                    "gencodeId": "ENSG00000000001.1",
                    "geneSymbol": "SYNTH1",
                    "median": 40.0,
                    "unit": "TPM",
                    "tissueSiteDetailId": "Liver"
                })],
                800,
                0,
                4,
            ))),
        )
        .route(
            "/association/egene",
            get(capture(paged(
                vec![json!({
                    "gencodeId": "ENSG00000000001.1",
                    "geneSymbol": "SYNTH1",
                    "tissueSiteDetailId": "Liver",
                    "pValue": 0.001,
                    "empiricalPValue": 0.01,
                    "qValue": 0.02,
                    "log2AllelicFoldChange": 0.4
                })],
                1,
                0,
                1,
            ))),
        )
        .route(
            "/association/singleTissueEqtl",
            get(capture(paged(
                vec![json!({
                    "gencodeId": "ENSG00000000001.1",
                    "geneSymbol": "SYNTH1",
                    "variantId": "chr1_100_A_G_b38",
                    "snpId": "rs1",
                    "tissueSiteDetailId": "Liver",
                    "pValue": 1e-8,
                    "nes": 0.3
                })],
                1,
                0,
                1,
            ))),
        )
        .route(
            "/association/metasoft",
            get(capture(paged(
                vec![json!({
                    "gencodeId": "ENSG00000000001.1",
                    "variantId": "chr1_100_A_G_b38",
                    "metaP": 0.01,
                    "tissues": {"Liver": {"mValue": 0.9, "nes": 0.3, "pValue": 0.01, "se": 0.1}}
                })],
                1,
                0,
                1,
            ))),
        )
        .route(
            "/association/dyneqtl",
            get(capture(json!({
                "gencodeId": "ENSG00000000001.1",
                "geneSymbol": "SYNTH1",
                "variantId": "chr1_100_A_G_b38",
                "tissueSiteDetailId": "Liver",
                "pValue": 0.2,
                "nes": 0.1,
                "tStatistic": 1.1,
                "maf": 0.25,
                "error": 0.05,
                "hetCount": 1,
                "homoAltCount": 1,
                "homoRefCount": 1,
                "genotypes": [2, 0, 1],
                "data": [9.0, 1.0, 4.0]
            }))),
        )
        .route(
            "/dataset/sample",
            get(capture(paged(
                vec![json!({
                    "sampleId": "GTEX-AAAA-0001-SM-XXXXX",
                    "subjectId": "GTEX-AAAA",
                    "tissueSiteDetailId": "Liver",
                    "dataType": "RNASEQ",
                    "rin": 7.2
                })],
                1,
                0,
                1,
            ))),
        );
    let (bio, server) = serve(app).await;
    let resolved = bio
        .call(
            "gtex_resolve_genes",
            &json!({"gene_ids": ["SYNTH1", "MISSING1"], "dataset_id": "gtex_v10"}),
        )
        .await
        .unwrap();
    let summary = bio
        .call("gtex_expression_summary", &json!({"gene": "SYNTH1"}))
        .await
        .unwrap();
    let median = bio
        .call(
            "gtex_median_expression",
            &json!({"gencode_ids": ["ENSG00000000001.1"], "tissue_site_detail_ids": ["Liver"]}),
        )
        .await
        .unwrap();
    let expression = bio
        .call(
            "gtex_gene_expression",
            &json!({"gencode_id": "ENSG00000000001.1", "tissue_site_detail_ids": ["Liver"]}),
        )
        .await
        .unwrap();
    let top = bio
        .call(
            "gtex_top_expressed_genes",
            &json!({"tissue_site_detail_id": "Liver", "n": 1, "filter_mt_gene": false}),
        )
        .await
        .unwrap();
    let egenes = bio
        .call(
            "gtex_eqtl_genes",
            &json!({"tissue_site_detail_id": "Liver", "max_genes": 10}),
        )
        .await
        .unwrap();
    let eqtls = bio
        .call(
            "gtex_single_tissue_eqtls",
            &json!({
                "gencode_id": "ENSG00000000001.1",
                "variant_id": "chr1_100_A_G_b38",
                "tissue_site_detail_id": "Liver"
            }),
        )
        .await
        .unwrap();
    let meta = bio
        .call(
            "gtex_multi_tissue_eqtls",
            &json!({"gencode_id": "ENSG00000000001.1", "variant_id": "chr1_100_A_G_b38"}),
        )
        .await
        .unwrap();
    let dyn_eqtl = bio
        .call(
            "gtex_calculate_eqtl",
            &json!({
                "gencode_id": "ENSG00000000001.1",
                "variant_id": "chr1_100_A_G_b38",
                "tissue_site_detail_id": "Liver"
            }),
        )
        .await
        .unwrap();
    let samples = bio
        .call(
            "gtex_sample_info",
            &json!({
                "tissue_site_detail_id": "Liver",
                "data_type": "RNASEQ",
                "subject_id": "GTEX-AAAA",
                "max_samples": 10
            }),
        )
        .await
        .unwrap();
    server.abort();
    let traffic = captured.lock().unwrap().join("\n");
    assert!(traffic.contains("/reference/gene"), "{traffic}");
    assert!(traffic.contains("gencodeVersion=v39"), "{traffic}");
    assert!(traffic.contains("geneId=SYNTH1"), "{traffic}");
    assert!(traffic.contains("gencodeId=ENSG00000000001.1"), "{traffic}");
    assert!(traffic.contains("filterMtGene=false"), "{traffic}");
    assert!(traffic.contains("dataType=RNASEQ"), "{traffic}");
    assert!(traffic.contains("subjectId=GTEX-AAAA"), "{traffic}");
    assert_eq!(resolved["missing_ids"], json!(["MISSING1"]));
    assert_eq!(
        resolved["genes"][0]["url"],
        "https://gtexportal.org/home/gene/SYNTH1"
    );
    assert_eq!(summary["gene"]["gencodeId"], "ENSG00000000001.1");
    assert_eq!(summary["tissues_ranked"][0]["tissueSiteDetailId"], "Liver");
    assert_eq!(median["medians"][0]["median"], 12.5);
    assert_eq!(expression["tissues"][0]["n_samples"], 3);
    assert_eq!(top["total_genes_in_ranking"], 800);
    assert_eq!(top["returned"], 1);
    assert_eq!(top["truncated"], true);
    assert_eq!(egenes["egenes"][0]["geneSymbol"], "SYNTH1");
    assert_eq!(eqtls["eqtls"][0]["variantId"], "chr1_100_A_G_b38");
    assert_eq!(meta["associations"][0]["metaP"], 0.01);
    assert_eq!(dyn_eqtl["genotypes"], json!([0, 1, 2]));
    assert_eq!(dyn_eqtl["data"], json!([1.0, 4.0, 9.0]));
    assert_eq!(dyn_eqtl["source_url"], GTEX_API);
    assert_eq!(samples["samples"][0]["sampleId"], "GTEX-AAAA-0001-SM-XXXXX");
}

#[tokio::test]
async fn walks_pages_and_reports_truncation_against_the_api_total() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = captured.clone();
    let app = Router::new().route(
        "/dataset/sample",
        get(
            move |uri: Uri, Query(params): Query<HashMap<String, String>>| {
                let seen = seen.clone();
                async move {
                    seen.lock().unwrap().push(uri.to_string());
                    let page: u64 = params.get("page").and_then(|v| v.parse().ok()).unwrap_or(0);
                    if page == 0 {
                        axum::Json(paged(
                            vec![
                                json!({"sampleId": "GTEX-AAAA-0001-SM-XXXXX"}),
                                json!({"sampleId": "GTEX-AAAA-0002-SM-XXXXX"}),
                            ],
                            3,
                            0,
                            2,
                        ))
                        .into_response()
                    } else {
                        axum::Json(paged(
                            vec![json!({"sampleId": "GTEX-BBBB-0003-SM-XXXXX"})],
                            3,
                            1,
                            2,
                        ))
                        .into_response()
                    }
                }
            },
        ),
    );
    let (bio, server) = serve(app).await;
    let all = bio
        .call("gtex_sample_info", &json!({"max_samples": 50}))
        .await
        .unwrap();
    let capped = bio
        .call("gtex_sample_info", &json!({"max_samples": 2}))
        .await
        .unwrap();
    server.abort();
    let traffic = captured.lock().unwrap().join("\n");
    assert!(traffic.contains("page=0"), "{traffic}");
    assert!(traffic.contains("page=1"), "{traffic}");
    assert_eq!(all["total"], 3);
    assert_eq!(all["returned"], 3);
    assert_eq!(all["truncated"], false);
    assert_eq!(capped["total"], 3);
    assert_eq!(capped["returned"], 2);
    assert_eq!(capped["truncated"], true);
}

#[tokio::test]
async fn unversioned_ensembl_ids_get_a_hint_on_empty_expression_pages() {
    let app = Router::new().route(
        "/expression/medianGeneExpression",
        get(|| async { axum::Json(paged(vec![], 0, 0, 0)) }),
    );
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "gtex_median_expression",
            &json!({"gencode_ids": ["ENSG00000000001"]}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["total"], 0);
    assert_eq!(result["returned"], 0);
    let hint = result["hint"].as_str().unwrap();
    assert!(hint.contains("versioned GENCODE"), "{hint}");
    assert!(hint.contains("ENSG00000000001"), "{hint}");
}

#[tokio::test]
async fn rejects_rate_limits_malformed_json_html_and_count_mismatches() {
    for (status, body, expected) in [
        (
            StatusCode::TOO_MANY_REQUESTS,
            "secret-token".into(),
            "HTTP 429",
        ),
        (StatusCode::OK, "{not-json".into(), "invalid JSON"),
        (
            StatusCode::OK,
            "<!doctype html><html><body>portal</body></html>".into(),
            "HTML",
        ),
        (
            StatusCode::OK,
            paged(vec![json!({"tissueSiteDetailId": "Liver"})], 5, 0, 1).to_string(),
            "page count",
        ),
    ] {
        let app = Router::new().route(
            "/dataset/tissueSiteDetail",
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
            .call("gtex_tissue_sites", &json!({}))
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
        "/dataset/tissueSiteDetail",
        get(|| async { (StatusCode::OK, " ".repeat(MAX_RESPONSE + 1)).into_response() }),
    );
    let (bio, server) = serve(app).await;
    let error = bio
        .call("gtex_tissue_sites", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("exceeded 4 MiB"), "{error}");
}

#[tokio::test]
async fn unknown_and_gated_tools_are_rejected() {
    let (bio, server) = serve(Router::new()).await;
    for name in [
        "gtex_not_a_tool",
        "panglaodb_marker_genes",
        "panglaodb_options",
        "panglaodb_cell_types_for_gene",
    ] {
        let error = bio.call(name, &json!({})).await.unwrap_err().to_string();
        assert!(
            error.contains("unknown native biological tool"),
            "{name}: {error}"
        );
    }
    let missing_eqtl = bio
        .call(
            "gtex_single_tissue_eqtls",
            &json!({"tissue_site_detail_id": "Liver"}),
        )
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(missing_eqtl.contains("gencode_id"), "{missing_eqtl}");
}
