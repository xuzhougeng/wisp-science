use super::*;
use crate::http::{Http, MAX_RESPONSE};
use axum::{
    http::{StatusCode, Uri},
    response::IntoResponse,
    routing::get,
    Router,
};
use serde_json::json;
use std::sync::{Arc, Mutex as StdMutex};

fn test_bio(base: &str) -> NativeBio {
    let base = base.trim_end_matches('/').to_string();
    NativeBio::test_client(
        &[
            ("ENCODE_BASE_URL".into(), base.clone()),
            ("JASPAR_BASE_URL".into(), format!("{base}/jaspar")),
            ("UNIBIND_BASE_URL".into(), format!("{base}/unibind")),
            ("UCSC_BASE_URL".into(), format!("{base}/ucsc")),
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

fn experiment_hit() -> Value {
    json!({
        "accession": "ENCSR000AKP",
        "assay_title": "TF ChIP-seq",
        "assay_term_name": "ChIP-seq",
        "target": {"label": "CTCF"},
        "biosample_ontology": {"term_name": "K562", "classification": "cell line"},
        "status": "released",
        "date_released": "2011-06-10",
        "lab": {"title": "ENCODE lab"}
    })
}

fn matrix_hit() -> Value {
    json!({
        "matrix_id": "MA0002.2",
        "name": "RUNX1",
        "collection": "CORE",
        "base_id": "MA0002",
        "version": "2",
        "sequence_logo": "https://jaspar.elixir.no/static/logos/svg/MA0002.2.svg",
        "url": "https://jaspar.elixir.no/api/v1/matrix/MA0002.2/",
        "pfm": {"A": [1, 2], "C": [3, 4], "G": [5, 6], "T": [7, 8]},
        "species": [{"tax_id": "9606", "name": "Homo sapiens"}],
        "tax_group": "vertebrates"
    })
}

fn dataset_hit() -> Value {
    json!({
        "tf_name": "CTCF",
        "total_peaks": 42,
        "url": "https://unibind.uio.no/api/v1/datasets/ENCSR000AUE.A549_lung_carcinoma.CTCF/"
    })
}

fn dispatch(path: &str, query: &str) -> axum::response::Response {
    if path == "/search/" {
        if query.contains("type=File") {
            return axum::Json(json!({
                "notification": "Success",
                "@id": "/search/?type=File",
                "total": 2,
                "@graph": [{
                    "accession": "ENCFF002JUR",
                    "file_format": "bam",
                    "output_type": "alignments",
                    "assay_term_name": "ChIP-seq",
                    "assembly": "GRCh38",
                    "dataset": "/experiments/ENCSR000AKP/",
                    "status": "released",
                    "file_size": 1000,
                    "date_created": "2012-01-01"
                }]
            }))
            .into_response();
        }
        if query.contains("type=Biosample") {
            return axum::Json(json!({
                "notification": "Success",
                "@id": "/search/?type=Biosample",
                "total": 1,
                "@graph": [{
                    "accession": "ENCBS013JZP",
                    "biosample_ontology": {"term_name": "K562", "classification": "cell line"},
                    "organism": {"scientific_name": "Homo sapiens"},
                    "status": "released",
                    "lab": {"title": "ENCODE lab"},
                    "summary": "K562 cell line",
                    "date_created": "2013-01-01"
                }]
            }))
            .into_response();
        }
        return axum::Json(json!({
            "notification": "Success",
            "@id": "/search/?type=Experiment",
            "total": 3,
            "@graph": [experiment_hit()]
        }))
        .into_response();
    }
    if path.starts_with("/experiments/") {
        let mut doc = experiment_hit();
        doc["description"] = json!("synthetic CTCF ChIP-seq");
        doc["uuid"] = json!("11111111-1111-1111-1111-111111111111");
        doc["bio_replicate_count"] = json!(2);
        return axum::Json(doc).into_response();
    }
    if path.starts_with("/files/") {
        return axum::Json(json!({
            "accession": "ENCFF002JUR",
            "status": "released",
            "file_format": "bam",
            "output_type": "alignments",
            "assay_term_name": "ChIP-seq",
            "assembly": "GRCh38",
            "dataset": "/experiments/ENCSR000AKP/",
            "file_size": 1000,
            "md5sum": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "href": "/files/ENCFF002JUR/@@download/ENCFF002JUR.bam",
            "uuid": "22222222-2222-2222-2222-222222222222"
        }))
        .into_response();
    }
    if path.starts_with("/biosamples/") {
        return axum::Json(json!({
            "accession": "ENCBS013JZP",
            "status": "released",
            "biosample_ontology": {"term_name": "K562", "classification": "cell line"},
            "organism": {"scientific_name": "Homo sapiens"},
            "donor": {"accession": "ENCDO000AAE"},
            "lab": {"title": "ENCODE lab"},
            "summary": "K562 cell line",
            "uuid": "33333333-3333-3333-3333-333333333333"
        }))
        .into_response();
    }
    if path == "/jaspar/matrix/" {
        return axum::Json(json!({
            "count": 40,
            "next": "https://jaspar.elixir.no/api/v1/matrix/?page=2",
            "previous": null,
            "results": [matrix_hit()]
        }))
        .into_response();
    }
    if path.ends_with("/versions/") {
        return axum::Json(json!({
            "count": 2,
            "next": null,
            "results": [
                {"matrix_id": "MA0002.1", "name": "RUNX1", "collection": "CORE"},
                {"matrix_id": "MA0002.2", "name": "RUNX1", "collection": "CORE"}
            ]
        }))
        .into_response();
    }
    if path.starts_with("/jaspar/matrix/") {
        return axum::Json(matrix_hit()).into_response();
    }
    if path == "/jaspar/species/" {
        return axum::Json(json!({
            "count": 1,
            "next": null,
            "results": [{"tax_id": "9606", "species": "Homo sapiens"}]
        }))
        .into_response();
    }
    if path == "/jaspar/taxon/" {
        return axum::Json(json!({
            "count": 1,
            "next": null,
            "results": [{"name": "vertebrates"}]
        }))
        .into_response();
    }
    if path == "/jaspar/collections/" {
        return axum::Json(json!({
            "count": 1,
            "next": null,
            "results": [{"name": "CORE"}]
        }))
        .into_response();
    }
    if path == "/jaspar/releases/" {
        return axum::Json(json!({
            "count": 1,
            "next": null,
            "results": [{"release_number": 2024, "year": 2024, "active": true}]
        }))
        .into_response();
    }
    if path == "/unibind/datasets/" {
        return axum::Json(json!({
            "count": 9,
            "next": "https://unibind.uio.no/api/v1/datasets/?page=2",
            "previous": null,
            "results": [dataset_hit()]
        }))
        .into_response();
    }
    if path.starts_with("/unibind/datasets/") {
        return axum::Json(json!({
            "tf_id": "ENCSR000AUE.A549_lung_carcinoma.CTCF",
            "tf_name": "CTCF",
            "identifier": ["ENCSR000AUE"],
            "cell_line": ["A549 lung carcinoma"],
            "biological_condition": ["none"],
            "jaspar_id": ["MA0139.1"],
            "prediction_models": ["DAMO"],
            "total_peaks": 42,
            "tfbs": [{
                "DAMO": [{
                    "jaspar_id": "MA0139.1",
                    "jaspar_version": "1",
                    "total_tfbs": 17,
                    "score_threshold": 0.8,
                    "distance_threshold": 50,
                    "adj_centrimo_pvalue": 0.001,
                    "bed_url": "https://unibind.uio.no/static/ctcf.bed",
                    "fasta_url": "https://unibind.uio.no/static/ctcf.fa"
                }]
            }]
        }))
        .into_response();
    }
    if path == "/ucsc/getData/track" {
        return axum::Json(json!({
            "maxItemsLimit": true,
            "UniBind": [{
                "chrom": "chr1",
                "chromStart": 100,
                "chromEnd": 120,
                "strand": "+",
                "score": 400,
                "name": "ENCSR000AUE_A549-lung-carcinoma_CTCF_MA0139.1"
            }, {
                "chrom": "chr1",
                "chromStart": 200,
                "chromEnd": 220,
                "strand": "-",
                "name": "ENCSR000AUE_A549-lung-carcinoma_FOXA1_MA0148.4"
            }]
        }))
        .into_response();
    }
    (StatusCode::NOT_FOUND, "missing").into_response()
}

#[test]
fn catalog_registers_sixteen_regulation_tools() {
    let names: Vec<_> = catalog()
        .into_iter()
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        names,
        vec![
            ("regulation", "encode_search_experiments".into()),
            ("regulation", "encode_search_biosamples".into()),
            ("regulation", "encode_list_files".into()),
            ("regulation", "encode_get_experiment".into()),
            ("regulation", "encode_get_file".into()),
            ("regulation", "encode_get_biosample".into()),
            ("regulation", "jaspar_get_matrix".into()),
            ("regulation", "jaspar_matrix_versions".into()),
            ("regulation", "jaspar_list_matrices".into()),
            ("regulation", "jaspar_list_species".into()),
            ("regulation", "jaspar_list_taxa".into()),
            ("regulation", "jaspar_list_collections".into()),
            ("regulation", "jaspar_list_releases".into()),
            ("regulation", "unibind_search_tfbs".into()),
            ("regulation", "unibind_get_dataset".into()),
            ("regulation", "unibind_tfbs_in_region".into()),
        ]
    );
    assert!(crate::contains_tool("encode_search_experiments"));
    assert_eq!(
        crate::domain_for_tool("jaspar_get_matrix"),
        Some("regulation")
    );
    assert!(crate::package_selects("mcp_regulation", "regulation"));
    assert!(crate::selected_by_package("mcp_regulation"));
}

#[tokio::test]
async fn rejects_unbounded_or_unknown_arguments_before_http() {
    let bio = NativeBio::new(&[]).unwrap();
    for (name, args) in [
        ("encode_get_experiment", json!({"accession": "ENCFF000AAA"})),
        (
            "encode_get_experiment",
            json!({"accession": "ENCSR000AKP", "api_key": "secret"}),
        ),
        ("encode_search_experiments", json!({"max_rows": 0})),
        ("encode_search_experiments", json!({"max_rows": 101})),
        (
            "encode_search_experiments",
            json!({"extra_filters": {"format": "json"}}),
        ),
        (
            "encode_search_experiments",
            json!({"date_released_before": "2024/01/01"}),
        ),
        ("jaspar_get_matrix", json!({"matrix_id": "MA0002"})),
        ("jaspar_list_matrices", json!({"version": "all"})),
        ("jaspar_list_species", json!({"search": "human"})),
        ("unibind_get_dataset", json!({"tf_id": "nope"})),
        (
            "unibind_tfbs_in_region",
            json!({"genome": "hg19", "chrom": "chr1", "start": 0, "end": 10}),
        ),
        (
            "unibind_tfbs_in_region",
            json!({"genome": "hg38", "chrom": "chr1", "start": 50, "end": 10}),
        ),
        (
            "unibind_tfbs_in_region",
            json!({"genome": "hg38", "chrom": "chr1", "start": 0, "end": 1_000_001}),
        ),
        (
            "unibind_tfbs_in_region",
            json!({"genome": "spo2", "chrom": "chr1", "start": 0, "end": 10, "collection": "Robust"}),
        ),
    ] {
        let error = bio.call(name, &args).await.unwrap_err().to_string();
        assert!(
            !error.contains("secret"),
            "{name} {args} leaked secret: {error}"
        );
        assert!(
            error.contains("invalid")
                || error.contains("must")
                || error.contains("takes no")
                || error.contains("cannot set")
                || error.contains("not in the UniBind")
                || error.contains("exceeds"),
            "{name} {args} → {error}"
        );
    }
}

#[tokio::test]
async fn encode_search_encodes_filters_and_reports_totals_and_source_urls() {
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let traffic = seen.clone();
    let app = Router::new().fallback(get(move |uri: Uri| {
        let seen = seen.clone();
        async move {
            seen.lock()
                .unwrap()
                .push(format!("{}?{}", uri.path(), uri.query().unwrap_or("")));
            dispatch(uri.path(), uri.query().unwrap_or(""))
        }
    }));
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "encode_search_experiments",
            &json!({
                "assay_title": "TF ChIP-seq",
                "target": "CTCF",
                "organism": "Homo sapiens",
                "date_released_before": "2020-01-01",
                "extra_filters": {"biosample_ontology.term_name": "K562"},
                "max_rows": 1
            }),
        )
        .await
        .unwrap();
    server.abort();
    let query = traffic.lock().unwrap().join("\n");
    assert!(query.contains("/search/"), "{query}");
    assert!(query.contains("type=Experiment"), "{query}");
    assert!(query.contains("limit=1"), "{query}");
    assert!(query.contains("assay_title=TF"), "{query}");
    assert!(query.contains("target.label=CTCF"), "{query}");
    assert!(
        query.contains("date_released=lte%3A2020-01-01")
            || query.contains("date_released=lte:2020-01-01"),
        "{query}"
    );
    assert!(
        query.contains("biosample_ontology.term_name=K562"),
        "{query}"
    );
    assert_eq!(result["source"], "ENCODE");
    assert_eq!(result["total"], 3);
    assert_eq!(result["returned"], 1);
    assert_eq!(result["truncated"], true);
    assert_eq!(result["has_more"], true);
    assert_eq!(
        result["records"][0]["url"],
        "https://www.encodeproject.org/experiments/ENCSR000AKP/"
    );
    assert!(result["source_url"]
        .as_str()
        .unwrap()
        .starts_with("https://www.encodeproject.org/"));
}

#[tokio::test]
async fn remaining_tools_dispatch_through_native_bio_call() {
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let traffic = seen.clone();
    let app = Router::new().fallback(get(move |uri: Uri| {
        let seen = seen.clone();
        async move {
            seen.lock()
                .unwrap()
                .push(format!("{}?{}", uri.path(), uri.query().unwrap_or("")));
            dispatch(uri.path(), uri.query().unwrap_or(""))
        }
    }));
    let (bio, server) = serve(app).await;

    let biosamples = bio
        .call(
            "encode_search_biosamples",
            &json!({"term_name": "K562", "max_rows": 10}),
        )
        .await
        .unwrap();
    let files = bio
        .call(
            "encode_list_files",
            &json!({"file_format": "bam", "assay_term_name": "ChIP-seq", "max_rows": 10}),
        )
        .await
        .unwrap();
    let experiment = bio
        .call(
            "encode_get_experiment",
            &json!({"accession": "ENCSR000AKP"}),
        )
        .await
        .unwrap();
    let file = bio
        .call("encode_get_file", &json!({"accession": "ENCFF002JUR"}))
        .await
        .unwrap();
    let biosample = bio
        .call("encode_get_biosample", &json!({"accession": "ENCBS013JZP"}))
        .await
        .unwrap();
    let matrix = bio
        .call("jaspar_get_matrix", &json!({"matrix_id": "MA0002.2"}))
        .await
        .unwrap();
    let versions = bio
        .call("jaspar_matrix_versions", &json!({"base_id": "MA0002.2"}))
        .await
        .unwrap();
    let matrices = bio
        .call(
            "jaspar_list_matrices",
            &json!({"collection": "CORE", "tax_id": 9606, "max_rows": 25}),
        )
        .await
        .unwrap();
    let species = bio.call("jaspar_list_species", &json!({})).await.unwrap();
    let taxa = bio.call("jaspar_list_taxa", &json!({})).await.unwrap();
    let collections = bio
        .call("jaspar_list_collections", &json!({}))
        .await
        .unwrap();
    let releases = bio.call("jaspar_list_releases", &json!({})).await.unwrap();
    let datasets = bio
        .call(
            "unibind_search_tfbs",
            &json!({"tf_name": "CTCF", "collection": "Robust", "max_rows": 5}),
        )
        .await
        .unwrap();
    let dataset = bio
        .call(
            "unibind_get_dataset",
            &json!({"tf_id": "ENCSR000AUE.A549_lung_carcinoma.CTCF"}),
        )
        .await
        .unwrap();
    let sites = bio
        .call(
            "unibind_tfbs_in_region",
            &json!({
                "genome": "hg38",
                "chrom": "chr1",
                "start": 0,
                "end": 1000,
                "tf_name": "CTCF",
                "max_sites": 10
            }),
        )
        .await
        .unwrap();
    server.abort();

    let query = traffic.lock().unwrap().join("\n");
    assert!(query.contains("type=Biosample"), "{query}");
    assert!(query.contains("type=File"), "{query}");
    assert!(query.contains("file_format=bam"), "{query}");
    assert!(query.contains("/experiments/ENCSR000AKP/"), "{query}");
    assert!(query.contains("/jaspar/matrix/MA0002.2/"), "{query}");
    assert!(query.contains("/jaspar/matrix/MA0002/versions/"), "{query}");
    assert!(query.contains("tax_id=9606"), "{query}");
    assert!(query.contains("/unibind/datasets/?"), "{query}");
    assert!(query.contains("tf_name=CTCF"), "{query}");
    assert!(query.contains("/ucsc/getData/track"), "{query}");
    assert!(query.contains("maxItemsOutput=5000"), "{query}");
    assert!(
        query.contains("UniBind_hubs_Robust") && query.contains("hub.txt"),
        "{query}"
    );

    assert_eq!(biosamples["records"][0]["accession"], "ENCBS013JZP");
    assert_eq!(files["records"][0]["file_format"], "bam");
    assert_eq!(experiment["record_type"], "experiment");
    assert_eq!(experiment["target_label"], "CTCF");
    assert_eq!(
        experiment["source_url"],
        "https://www.encodeproject.org/experiments/ENCSR000AKP/"
    );
    assert_eq!(
        file["download_url"],
        "https://www.encodeproject.org/files/ENCFF002JUR/@@download/ENCFF002JUR.bam"
    );
    assert_eq!(biosample["organism"], "Homo sapiens");
    assert_eq!(matrix["matrix_id"], "MA0002.2");
    assert_eq!(matrix["pfm"]["A"], json!([1, 2]));
    assert_eq!(versions["base_id"], "MA0002");
    assert_eq!(versions["count"], 2);
    assert_eq!(matrices["count"], 40);
    assert_eq!(matrices["truncated"], true);
    assert_eq!(species["species"][0]["tax_id"], "9606");
    assert_eq!(taxa["taxa"][0]["name"], "vertebrates");
    assert_eq!(collections["collections"][0]["name"], "CORE");
    assert_eq!(releases["releases"][0]["active"], true);
    assert_eq!(datasets["total"], 9);
    assert_eq!(
        datasets["datasets"][0]["tf_id"],
        "ENCSR000AUE.A549_lung_carcinoma.CTCF"
    );
    assert_eq!(dataset["n_models"], 1);
    assert_eq!(dataset["models"][0]["total_tfbs"], 17);
    assert_eq!(sites["returned"], 1);
    assert_eq!(sites["n_matching"], 1);
    assert_eq!(sites["items_scanned"], 2);
    assert_eq!(sites["region_scan_complete"], false);
    assert_eq!(sites["truncated"], true);
    assert_eq!(sites["sites"][0]["tf_name"], "CTCF");
    assert_eq!(sites["sites"][0]["jaspar_matrix"], "MA0139.1");
}

#[tokio::test]
async fn encode_zero_hit_search_is_empty_not_an_error() {
    let app = Router::new().route(
        "/search/",
        get(|| async { (StatusCode::NOT_FOUND, "secret-token").into_response() }),
    );
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "encode_search_experiments",
            &json!({"assay_title": "no-such-assay"}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["total"], 0);
    assert_eq!(result["returned"], 0);
    assert_eq!(result["truncated"], false);
    assert_eq!(result["records"], json!([]));
}

#[tokio::test]
async fn missing_objects_rate_limits_html_and_oversize_fail_without_echoing_secrets() {
    for (path, status, body, tool, args, expected) in [
        (
            "/experiments/ENCSR000AKP/",
            StatusCode::NOT_FOUND,
            "secret-token",
            "encode_get_experiment",
            json!({"accession": "ENCSR000AKP"}),
            "no experiments record",
        ),
        (
            "/search/",
            StatusCode::TOO_MANY_REQUESTS,
            "secret-token",
            "encode_search_experiments",
            json!({"target": "CTCF"}),
            "HTTP 429",
        ),
        (
            "/jaspar/matrix/MA0002.2/",
            StatusCode::OK,
            "<!doctype html><html>secret-token</html>",
            "jaspar_get_matrix",
            json!({"matrix_id": "MA0002.2"}),
            "HTML",
        ),
        (
            "/jaspar/matrix/MA0002.2/",
            StatusCode::OK,
            "{not-json secret-token",
            "jaspar_get_matrix",
            json!({"matrix_id": "MA0002.2"}),
            "invalid JSON",
        ),
        (
            "/unibind/datasets/ENCSR000AUE.A549_lung_carcinoma.CTCF/",
            StatusCode::TOO_MANY_REQUESTS,
            "secret-token",
            "unibind_get_dataset",
            json!({"tf_id": "ENCSR000AUE.A549_lung_carcinoma.CTCF"}),
            "HTTP 429",
        ),
    ] {
        let path = path.to_string();
        let body = body.to_string();
        let app = Router::new().fallback(get(move |uri: Uri| {
            let path = path.clone();
            let body = body.clone();
            async move {
                if uri.path() == path {
                    (status, [("retry-after", "60")], body).into_response()
                } else {
                    StatusCode::NOT_FOUND.into_response()
                }
            }
        }));
        let (bio, server) = serve(app).await;
        let error = bio.call(tool, &args).await.unwrap_err().to_string();
        server.abort();
        assert!(
            error
                .to_ascii_lowercase()
                .contains(&expected.to_ascii_lowercase()),
            "{tool} {error} did not contain {expected}"
        );
        assert!(!error.contains("secret-token"), "{error}");
    }

    let app = Router::new().route(
        "/search/",
        get(|| async { (StatusCode::OK, " ".repeat(MAX_RESPONSE + 1)).into_response() }),
    );
    let (bio, server) = serve(app).await;
    let error = bio
        .call("encode_search_experiments", &json!({"target": "CTCF"}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("exceeded 4 MiB"), "{error}");
}

#[tokio::test]
async fn jaspar_next_url_off_host_is_rejected() {
    let app = Router::new().route(
        "/jaspar/species/",
        get(|| async {
            axum::Json(json!({
                "count": 2,
                "next": "https://evil.example/api/v1/species/?page=2",
                "results": [{"tax_id": "9606", "species": "Homo sapiens"}]
            }))
        }),
    );
    let (bio, server) = serve(app).await;
    let error = bio
        .call("jaspar_list_species", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("not on the same API host"), "{error}");
}
