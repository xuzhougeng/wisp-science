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
use flate2::{write::GzEncoder, Compression};
use serde_json::json;
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex as StdMutex};

const MARKER_PATH: &str = "/markers/PanglaoDB_markers_27_Mar_2020.tsv.gz";
const MARKER_HEADER: &str = "species\tofficial gene symbol\tcell type\tnicknames\tubiquitousness index\tproduct description\tgene type\tcanonical marker\tgerm layer\torgan\tsensitivity_human\tsensitivity_mouse\tspecificity_human\tspecificity_mouse";

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

fn gzip_tsv(tsv: &str) -> (Vec<u8>, String) {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(tsv.as_bytes()).unwrap();
    let bytes = encoder.finish().unwrap();
    let digest = panglaodb::sha256_hex(&bytes);
    (bytes, digest)
}

fn invented_marker_tsv() -> String {
    let rows = [
        "Hs\tSYNTH1\tAlpha cell\tS1|na\t0.1\tsynthetic protein 1\tprotein-coding gene\t1\tEndoderm\tPancreas\t0.9\t0.8\t0.1\t0.2",
        "Mm\tSYNTH1\tAlpha cell\tS1\t0.2\tsynthetic protein 1\tprotein-coding gene\t1\tEndoderm\tPancreas\tNA\t0.7\tNA\t0.3",
        "Hs\tSYNTH2\tBeta cell\tS2\t0.3\tsynthetic protein 2\tprotein-coding gene\t0\tEndoderm\tPancreas\t0.4\t0.5\t0.6\t0.4",
        "Mm Hs\tSYNTH3\tT cell\tALIAS3\t0.05\tsynthetic protein 3\tprotein-coding gene\t1\tMesoderm\tImmune system\t0.95\t0.9\t0.05\t0.1",
        "Hs\tSYNTH4\tNeuron\tN4\t0.5\tsynthetic protein 4\tprotein-coding gene\t1\tEctoderm\tBrain\t0.2\tNA\t0.8\tNA",
        "4\tSYNTH5\tUnknown\tna\tNA\tsynthetic protein 5\tprotein-coding gene\t0\tNA\tNA\tNA\tNA\tNA\tNA",
        "Hs\tSYNTH6\tAlpha cell\tA6\t0.15\tsynthetic protein 6\tprotein-coding gene\t1\tEndoderm\tLiver\t0.85\t0.1\t0.2\t0.9",
        "Hs\tSYNTH7\tAlpha cell\tA7\t0.12\tsynthetic protein 7\tprotein-coding gene\t1\tEndoderm\tPancreas\t0.5\t0.5\t0.5\t0.5",
    ];
    format!("{MARKER_HEADER}\n{}\n", rows.join("\n"))
}

fn panglao_test_bio(base: &str, digest: &str) -> NativeBio {
    let base = base.trim_end_matches('/');
    NativeBio::test_client(
        &[
            (
                "PANGLAODB_MARKER_URL".into(),
                format!("{base}{MARKER_PATH}"),
            ),
            ("PANGLAODB_SHA256".into(), digest.into()),
        ],
        Http(reqwest::Client::builder().no_proxy().build().unwrap()),
    )
    .unwrap()
}

async fn panglao_serve(app: Router, digest: &str) -> (NativeBio, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (panglao_test_bio(&endpoint, digest), task)
}

fn marker_route(bytes: Vec<u8>) -> Router {
    Router::new().route(
        MARKER_PATH,
        get(move || {
            let bytes = bytes.clone();
            async move { bytes }
        }),
    )
}

async fn serve_invented_markers() -> (NativeBio, tokio::task::JoinHandle<()>, String) {
    let (bytes, digest) = gzip_tsv(&invented_marker_tsv());
    let (bio, server) = panglao_serve(marker_route(bytes), &digest).await;
    (bio, server, digest)
}

#[test]
fn catalog_registers_gtex_and_panglaodb_tools() {
    let tools = catalog();
    let names: Vec<_> = tools
        .iter()
        .map(|(domain, schema)| (*domain, schema.function.name.as_str()))
        .collect();
    assert_eq!(
        names,
        vec![
            ("expression", "gtex_calculate_eqtl"),
            ("expression", "gtex_dataset_info"),
            ("expression", "gtex_eqtl_genes"),
            ("expression", "gtex_expression_summary"),
            ("expression", "gtex_gene_expression"),
            ("expression", "gtex_median_expression"),
            ("expression", "gtex_multi_tissue_eqtls"),
            ("expression", "gtex_resolve_genes"),
            ("expression", "gtex_sample_info"),
            ("expression", "gtex_single_tissue_eqtls"),
            ("expression", "gtex_tissue_sites"),
            ("expression", "gtex_top_expressed_genes"),
            ("expression", "panglaodb_cell_types_for_gene"),
            ("expression", "panglaodb_marker_genes"),
            ("expression", "panglaodb_options"),
        ]
    );
    assert_eq!(tools.len(), 15);
    assert!(crate::contains_tool("gtex_tissue_sites"));
    assert_eq!(
        crate::domain_for_tool("gtex_median_expression"),
        Some("expression")
    );
    assert!(crate::package_selects("mcp_expression", "expression"));
    assert!(crate::selected_by_package("mcp_expression"));
    assert!(crate::contains_tool("panglaodb_marker_genes"));
    assert!(crate::contains_tool("panglaodb_options"));
    assert!(crate::contains_tool("panglaodb_cell_types_for_gene"));
    assert_eq!(
        crate::domain_for_tool("panglaodb_marker_genes"),
        Some("expression")
    );
    for (domain, schema) in &tools {
        if schema.function.name.starts_with("panglaodb_") {
            assert_eq!(*domain, "expression");
            assert!(
                schema.function.description.contains("27 Mar 2020"),
                "{}",
                schema.function.name
            );
            assert!(
                schema.function.description.contains("does not spoof"),
                "{}",
                schema.function.name
            );
            assert!(
                schema.function.description.contains("wisp-science"),
                "{}",
                schema.function.name
            );
        }
    }
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
    let error = bio
        .call("gtex_not_a_tool", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("unknown native biological tool"),
        "gtex_not_a_tool: {error}"
    );
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

#[test]
fn panglaodb_rejects_invalid_arguments() {
    assert!(serde_json::from_value::<panglaodb::MarkerGenes>(
        json!({"species": "Hs", "api_key": "secret"})
    )
    .is_err());
    assert!(
        serde_json::from_value::<panglaodb::PanglaoOptions>(json!({"api_key": "secret"})).is_err()
    );
    assert!(serde_json::from_value::<panglaodb::CellTypesForGene>(
        json!({"gene_symbol": "SYNTH1", "api_key": "secret"})
    )
    .is_err());
}

#[tokio::test]
async fn panglaodb_rejects_species_and_max_rows_bounds() {
    let bio = test_bio("http://127.0.0.1:1");
    let species = bio
        .call("panglaodb_marker_genes", &json!({"species": "Rn"}))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        species.contains("Hs") && species.contains("Mm"),
        "{species}"
    );
    assert!(!species.contains("connection failed"), "{species}");
    for max_rows in [0, 501] {
        let error = bio
            .call("panglaodb_marker_genes", &json!({"max_rows": max_rows}))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("max_rows"), "{error}");
        assert!(!error.contains("connection failed"), "{error}");
    }
}

#[tokio::test]
async fn panglaodb_marker_genes_filters_and_reports_truncation() {
    let (bio, server, _) = serve_invented_markers().await;
    let alpha = bio
        .call(
            "panglaodb_marker_genes",
            &json!({"cell_type": "alpha cell", "max_rows": 2}),
        )
        .await
        .unwrap();
    let pancreas_hs = bio
        .call(
            "panglaodb_marker_genes",
            &json!({
                "cell_type": "Alpha cell",
                "organ": "pancreas",
                "species": "Hs",
                "canonical_only": true,
                "sensitivity_min": 0.8
            }),
        )
        .await
        .unwrap();
    let dual = bio
        .call("panglaodb_marker_genes", &json!({"species": "Mm"}))
        .await
        .unwrap();
    let canonical = bio
        .call(
            "panglaodb_marker_genes",
            &json!({"canonical_only": true, "max_rows": 50}),
        )
        .await
        .unwrap();
    let thresholds = bio
        .call(
            "panglaodb_marker_genes",
            &json!({"sensitivity_min": 0.8, "specificity_max": 0.15}),
        )
        .await
        .unwrap();
    let unknown = bio
        .call("panglaodb_marker_genes", &json!({"cell_type": "Unknown"}))
        .await
        .unwrap();
    server.abort();

    assert_eq!(alpha["source"], "PanglaoDB");
    assert_eq!(
        alpha["source_url"],
        "https://panglaodb.se/markers/PanglaoDB_markers_27_Mar_2020.tsv.gz"
    );
    assert_eq!(alpha["total_matching"], 4);
    assert_eq!(alpha["returned"], 2);
    assert_eq!(alpha["truncated"], true);
    assert_eq!(alpha["markers"][0]["official_gene_symbol"], "SYNTH1");
    assert_eq!(alpha["markers"][0]["species"], "Hs");
    assert_eq!(alpha["markers"][1]["official_gene_symbol"], "SYNTH1");
    assert_eq!(alpha["markers"][1]["species"], "Mm");

    assert_eq!(pancreas_hs["total_matching"], 1);
    assert_eq!(pancreas_hs["returned"], 1);
    assert_eq!(pancreas_hs["truncated"], false);
    assert_eq!(pancreas_hs["markers"][0]["official_gene_symbol"], "SYNTH1");
    assert_eq!(pancreas_hs["markers"][0]["species"], "Hs");
    assert_eq!(pancreas_hs["markers"][0]["canonical_marker"], "1");

    let mm_symbols: Vec<_> = dual["markers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["official_gene_symbol"].as_str().unwrap())
        .collect();
    assert_eq!(mm_symbols, vec!["SYNTH1", "SYNTH3"]);

    let canonical_symbols: Vec<_> = canonical["markers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["official_gene_symbol"].as_str().unwrap().to_string())
        .collect();
    assert!(!canonical_symbols.iter().any(|s| s == "SYNTH2"));
    assert!(!canonical_symbols.iter().any(|s| s == "SYNTH5"));
    assert_eq!(canonical["truncated"], false);

    let threshold_symbols: Vec<_> = thresholds["markers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["official_gene_symbol"].as_str().unwrap())
        .collect();
    assert_eq!(threshold_symbols, vec!["SYNTH1", "SYNTH3"]);

    assert_eq!(unknown["total_matching"], 1);
    assert_eq!(unknown["markers"][0]["organ"], Value::Null);
    assert_eq!(unknown["markers"][0]["ubiquitousness_index"], Value::Null);
    assert_eq!(unknown["markers"][0]["species"], "4");
}

#[tokio::test]
async fn panglaodb_options_lists_organs_and_excludes_na() {
    let (bio, server, _) = serve_invented_markers().await;
    let options = bio.call("panglaodb_options", &json!({})).await.unwrap();
    server.abort();
    assert_eq!(options["source"], "PanglaoDB");
    assert_eq!(options["species"], json!(["4", "Hs", "Mm", "Mm Hs"]));
    assert_eq!(
        options["organs"],
        json!(["Brain", "Immune system", "Liver", "Pancreas"])
    );
    assert_eq!(
        options["cell_types"],
        json!(["Alpha cell", "Beta cell", "Neuron", "T cell", "Unknown"])
    );
    assert_eq!(options["n_organs"], 4);
    assert_eq!(options["n_cell_types"], 5);
    assert_eq!(
        options["cell_types_by_organ"]["Pancreas"],
        json!(["Alpha cell", "Beta cell"])
    );
    assert_eq!(options["cell_types_by_organ"]["Brain"], json!(["Neuron"]));
    assert!(options["cell_types_by_organ"].get("NA").is_none());
    let organs = options["organs"].as_array().unwrap();
    assert!(!organs.iter().any(|organ| organ == "NA"));
}

#[tokio::test]
async fn panglaodb_cell_types_for_gene_matches_official_and_synonym() {
    let (bio, server, _) = serve_invented_markers().await;
    let official = bio
        .call(
            "panglaodb_cell_types_for_gene",
            &json!({"gene_symbol": "synth1"}),
        )
        .await
        .unwrap();
    let synonym_off = bio
        .call(
            "panglaodb_cell_types_for_gene",
            &json!({"gene_symbol": "ALIAS3"}),
        )
        .await
        .unwrap();
    let synonym_on = bio
        .call(
            "panglaodb_cell_types_for_gene",
            &json!({"gene_symbol": "ALIAS3", "include_synonyms": true}),
        )
        .await
        .unwrap();
    let ignored_na = bio
        .call(
            "panglaodb_cell_types_for_gene",
            &json!({"gene_symbol": "na", "include_synonyms": true}),
        )
        .await
        .unwrap();
    let none = bio
        .call(
            "panglaodb_cell_types_for_gene",
            &json!({"gene_symbol": "NOSUCH"}),
        )
        .await
        .unwrap();
    server.abort();

    assert_eq!(official["total"], 2);
    assert_eq!(official["matches"][0]["matched_via"], "official symbol");
    assert_eq!(official["matches"][0]["official_gene_symbol"], "SYNTH1");
    assert_eq!(official["matches"][1]["species"], "Mm");
    assert_eq!(synonym_off["total"], 0);
    assert_eq!(synonym_off["matches"], json!([]));
    assert_eq!(synonym_on["total"], 1);
    assert_eq!(synonym_on["matches"][0]["matched_via"], "synonym");
    assert_eq!(synonym_on["matches"][0]["official_gene_symbol"], "SYNTH3");
    assert_eq!(ignored_na["total"], 0);
    assert_eq!(none["total"], 0);
    assert_eq!(none["matches"], json!([]));
}

#[tokio::test]
async fn panglaodb_rejects_wrong_header_checksum_mismatch_and_http_403() {
    let bad_header = invented_marker_tsv().replacen("official gene symbol", "gene symbol", 1);
    let (bad_bytes, bad_digest) = gzip_tsv(&bad_header);
    let (bio, server) = panglao_serve(marker_route(bad_bytes), &bad_digest).await;
    let header_error = bio
        .call("panglaodb_options", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(header_error.contains("unexpected header"), "{header_error}");

    let (good_bytes, _good_digest) = gzip_tsv(&invented_marker_tsv());
    let (bio, server) = panglao_serve(marker_route(good_bytes), &"0".repeat(64)).await;
    let checksum_error = bio
        .call("panglaodb_marker_genes", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(
        checksum_error.contains("checksum mismatch"),
        "{checksum_error}"
    );
    assert!(
        checksum_error.contains("refusing to parse"),
        "{checksum_error}"
    );

    let app = Router::new().route(
        MARKER_PATH,
        get(|| async { (StatusCode::FORBIDDEN, "secret-forbidden-body").into_response() }),
    );
    let (bio, server) = panglao_serve(app, &"0".repeat(64)).await;
    let forbidden = bio
        .call("panglaodb_options", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(forbidden.contains("403"), "{forbidden}");
    assert!(!forbidden.contains("secret-forbidden-body"), "{forbidden}");
}

#[tokio::test]
async fn panglaodb_oversized_gzip_is_rejected_via_max_response() {
    let app = Router::new().route(
        MARKER_PATH,
        get(|| async { (StatusCode::OK, " ".repeat(MAX_RESPONSE + 1)).into_response() }),
    );
    let (bio, server) = panglao_serve(app, &"0".repeat(64)).await;
    let error = bio
        .call("panglaodb_marker_genes", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("exceeded 4 MiB"), "{error}");
}
