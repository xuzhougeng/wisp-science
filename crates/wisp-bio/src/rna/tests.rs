use super::*;
use crate::http::{Http, MAX_RESPONSE};
use crate::NativeBio;
use axum::{
    extract::Path,
    http::{StatusCode, Uri},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde_json::json;
use std::sync::{Arc, Mutex as StdMutex};

fn family_json() -> Value {
    json!({
        "rfam": {
            "acc": "RF99999",
            "id": "synthRNA",
            "description": "Synthetic test family",
            "comment": "Invented fixture",
            "curation": {
                "author": "fixture",
                "seed_source": "synthetic",
                "type": "Gene; snRNA;",
                "structure_source": "Predicted",
                "num_seed": "3",
                "num_full": 12,
                "num_species": 4
            },
            "cm": {
                "threshold": {
                    "gathering": 20.0,
                    "trusted": 20.5,
                    "noise": 19.5
                }
            },
            "release": {"number": "15.0", "date": "2024-01-01"},
            "clan": {"acc": "CL99999", "id": "synthClan"}
        }
    })
}

fn stockholm() -> &'static str {
    "# STOCKHOLM 1.0\n#=GF ID synthRNA\nseq1    ACGUACGU\nseq2    AUGUACGU\n//\n"
}

fn fasta() -> &'static str {
    ">seq1 synthetic\nACGUACGU\n>seq2\nAUGUACGU\n"
}

fn cm_text() -> &'static str {
    "INFERNAL-1.1\nNAME     synthRNA\nACC      RF99999\nDESC     Synthetic\nSTATES   10\nNODES    4\nCLEN     8\nW        12\nALPH     RNA\nGA       20.00\nCM\n1 2 3\n//\n"
}

fn tree_text() -> &'static str {
    "(seq1:0.10,seq2:0.20)0.9:0.01;"
}

fn regions_tsv() -> &'static str {
    "# file built 00:00:00 01-Jan-2024\n# found 3 regions\nSYNTH000001\t25.0\t1\t80\tsynthetic locus\tTest species\t9606\nSYNTH000002\t22.1\t10\t90\tanother locus\tTest species\t9606\nSYNTH000003\t18.0\t5\t70\tthird locus\tTest species\t9606\n"
}

fn mapping_json() -> Value {
    json!({
        "mapping": [
            {
                "rfam_acc": "RF99999",
                "pdb_id": "9zzz",
                "chain": "B",
                "pdb_start": 20,
                "pdb_end": 40,
                "cm_start": 1,
                "cm_end": 21,
                "bit_score": 40.0,
                "evalue_score": "1.2e-8"
            },
            {
                "rfam_acc": "RF99999",
                "pdb_id": "1aaa",
                "chain": "A",
                "pdb_start": 4,
                "pdb_end": 46,
                "cm_start": 1,
                "cm_end": 43,
                "bit_score": 65.6,
                "evalue_score": "4.4e-20"
            }
        ]
    })
}

fn test_bio(base: &str) -> NativeBio {
    NativeBio::test_client(
        &[("RFAM_BASE_URL".into(), base.trim_end_matches('/').into())],
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

fn success_app() -> Router {
    Router::new()
        .route(
            "/family/{family}",
            get(|Path(family): Path<String>| async move {
                assert!(family == "RF99999" || family == "synthRNA");
                axum::Json(family_json())
            }),
        )
        .route(
            "/family/{family}/id",
            get(|Path(family): Path<String>| async move {
                assert_eq!(family, "RF99999");
                "synthRNA"
            }),
        )
        .route(
            "/family/{family}/acc",
            get(|Path(family): Path<String>| async move {
                assert_eq!(family, "synthRNA");
                "RF99999"
            }),
        )
        .route("/family/{family}/alignment", get(|| async { stockholm() }))
        .route(
            "/family/{family}/alignment/{fmt}",
            get(|Path((_, fmt)): Path<(String, String)>| async move {
                match fmt.as_str() {
                    "fasta" | "fastau" => fasta().to_string(),
                    _ => stockholm().to_string(),
                }
            }),
        )
        .route("/family/{family}/cm", get(|| async { cm_text() }))
        .route("/family/{family}/tree", get(|| async { tree_text() }))
        .route("/family/{family}/regions", get(|| async { regions_tsv() }))
        .route(
            "/family/{family}/structures",
            get(|| async { axum::Json(mapping_json()) }),
        )
        .route(
            "/search/sequence",
            post(|| async {
                axum::Json(json!({
                    "jobId": "synthetic-job",
                    "resultURL": "/search/job/synthetic-job"
                }))
            }),
        )
        .route(
            "/search/job/{job}",
            get(|Path(job): Path<String>| async move {
                assert_eq!(job, "synthetic-job");
                axum::Json(json!({
                    "jobId": "synthetic-job",
                    "searchSequence": "ACGUACGU",
                    "hits": {
                        "synthRNA": [{
                            "id": "synthRNA",
                            "acc": "RF99999",
                            "start": 1,
                            "end": 8,
                            "strand": "+",
                            "GC": 0.5,
                            "score": 20.0,
                            "E": 1e-5,
                            "alignment": {"user_seq": "should-be-dropped"}
                        }]
                    }
                }))
            }),
        )
}

#[test]
fn catalog_registers_nine_read_only_rna_tools() {
    let names: Vec<_> = catalog()
        .into_iter()
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        names,
        vec![
            ("rna", "accession_to_id".into()),
            ("rna", "get_covariance_model".into()),
            ("rna", "get_family".into()),
            ("rna", "get_seed_alignment".into()),
            ("rna", "get_sequence_regions".into()),
            ("rna", "get_structure_mapping".into()),
            ("rna", "get_tree".into()),
            ("rna", "id_to_accession".into()),
            ("rna", "search_sequence".into()),
        ]
    );
    assert!(crate::contains_tool("get_family"));
    assert_eq!(crate::domain_for_tool("get_family"), Some("rna"));
    assert_eq!(crate::domain_for_tool("search_sequence"), Some("rna"));
    assert!(crate::package_selects("mcp_rna", "rna"));
    assert!(crate::selected_by_package("mcp_rna"));
}

#[test]
fn rejects_unbounded_or_malformed_arguments() {
    assert!(require_family("").is_err());
    assert!(require_family("   ").is_err());
    assert!(require_family("RF00005/cm").is_err());
    assert!(require_family("../RF00005").is_err());
    assert!(require_family("tRNA family").is_err());
    assert!(require_family(&"x".repeat(FAMILY_MAX + 1)).is_err());
    assert_eq!(require_family(" 5S_rRNA ").unwrap(), "5S_rRNA");
    assert!(require_accession("tRNA").is_err());
    assert!(require_accession("RF12").is_err());
    assert_eq!(require_accession("RF99999").unwrap(), "RF99999");
    assert!(normalize_sequence("").is_err());
    assert!(normalize_sequence(">seq\nACGU").is_err());
    assert!(normalize_sequence("ACGX").is_err());
    assert!(normalize_sequence(&"A".repeat(SEARCH_MAX_NT + 1)).is_err());
    assert_eq!(normalize_sequence(" ac\nGU ").unwrap(), "acGU");
    assert!(bound_bytes(0).is_err());
    assert!(bound_bytes(MAX_BYTES + 1).is_err());
    assert!(bound_page(0, "max_regions").is_err());
    assert!(bound_page(MAX_PAGE + 1, "max_regions").is_err());
    assert!(bound_page(MAX_HITS + 1, "max_hits").is_err());
    assert!(bound_seconds(0.0, MIN_WAIT, MAX_WAIT, "max_wait_s").is_err());
    assert!(bound_seconds(46.0, MIN_WAIT, MAX_WAIT, "max_wait_s").is_err());
    assert!(
        serde_json::from_value::<FamilyQuery>(json!({"family": "RF99999", "api_key": "x"}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<AlignmentQuery>(json!({"family": "RF99999", "fmt": "xml"}))
            .is_err()
    );
}

#[test]
fn parsers_flatten_invented_rfam_payloads() {
    let record = family_record(&family_json()).unwrap();
    assert_eq!(record["rfam_acc"], "RF99999");
    assert_eq!(record["rfam_id"], "synthRNA");
    assert_eq!(record["num_seed"], 3);
    assert_eq!(record["num_full"], 12);
    assert_eq!(record["gathering_cutoff"], 20.0);
    assert_eq!(record["clan_acc"], "CL99999");
    assert!(family_record(&json!({"rfam": {"id": "x"}})).is_err());

    let parsed = parse_regions(regions_tsv()).unwrap();
    assert_eq!(parsed.declared_count, Some(3));
    assert_eq!(parsed.regions.len(), 3);
    assert_eq!(parsed.regions[0]["sequence_accession"], "SYNTH000001");
    assert_eq!(parsed.regions[0]["ncbi_tax_id"], "9606");

    assert_eq!(
        parse_stockholm_seq_names(stockholm()),
        vec!["seq1".to_string(), "seq2".to_string()]
    );
    assert_eq!(
        parse_fasta_seq_names(fasta()),
        vec!["seq1".to_string(), "seq2".to_string()]
    );
    let header = parse_cm_header(cm_text());
    assert_eq!(header["NAME"], "synthRNA");
    assert_eq!(header["CLEN"], 8);
    assert_eq!(header["ACC"], "RF99999");
    assert_eq!(count_newick_leaves(tree_text()), 2);
    assert_eq!(
        count_newick_leaves("((a:0.1,b:0.1)[&&NHX:S=x]:0.2,c:0.3);"),
        3
    );

    let mut rows = structure_rows(&mapping_json()).unwrap();
    rows.sort_by(|a, b| mapping_key(a).cmp(&mapping_key(b)));
    assert_eq!(rows[0]["pdb_id"], "1aaa");
    assert_eq!(
        project_mapping(&rows[0])["pdb_url"],
        "https://www.rcsb.org/structure/1AAA"
    );

    let (families, hits) = flatten_hits(&json!({
        "synthRNA": [{"id": "synthRNA", "acc": "RF99999", "start": 1, "alignment": {"x": 1}}]
    }))
    .unwrap();
    assert_eq!(families, vec!["synthRNA"]);
    assert_eq!(hits[0]["acc"], "RF99999");
    assert!(hits[0].get("alignment").is_none());
}

#[test]
fn result_url_must_stay_on_rfam_or_the_configured_host() {
    assert_eq!(
        resolve_result_url("http://127.0.0.1:9", "/search/job/a").unwrap(),
        "http://127.0.0.1:9/search/job/a"
    );
    assert!(resolve_result_url("http://127.0.0.1:9", "https://rfam.org/search/job/a").is_ok());
    assert!(resolve_result_url("http://127.0.0.1:9", "https://evil.example/search").is_err());
    assert!(resolve_result_url("http://127.0.0.1:9", "http://127.0.0.1:9/../etc").is_err());
}

#[tokio::test]
async fn get_family_reports_source_urls_and_flattened_fields() {
    let seen = Arc::new(StdMutex::new(String::new()));
    let captured = seen.clone();
    let app = Router::new().route(
        "/family/{family}",
        get(move |Path(family): Path<String>, uri: Uri| {
            let captured = captured.clone();
            async move {
                *captured.lock().unwrap() = format!("{family} {uri}");
                axum::Json(family_json())
            }
        }),
    );
    let (bio, server) = serve(app).await;
    let result = bio
        .call("get_family", &json!({"family": "synthRNA"}))
        .await
        .unwrap();
    server.abort();
    let traffic = seen.lock().unwrap().clone();
    assert!(traffic.contains("synthRNA"), "{traffic}");
    assert!(
        traffic.contains("content-type=application%2Fjson")
            || traffic.contains("content-type=application/json"),
        "{traffic}"
    );
    assert_eq!(result["source"], "Rfam");
    assert_eq!(result["source_url"], RFAM_PUBLIC);
    assert_eq!(result["family_url"], "https://rfam.org/family/RF99999");
    assert_eq!(result["rfam_acc"], "RF99999");
    assert_eq!(result["rfam_id"], "synthRNA");
    assert_eq!(result["num_seed"], 3);
    assert_eq!(result["gathering_cutoff"], 20.0);
    assert_eq!(result["query"]["family"], "synthRNA");
}

#[tokio::test]
async fn remaining_tools_dispatch_through_native_bio_call() {
    let (bio, server) = serve(success_app()).await;
    let alignment = bio
        .call(
            "get_seed_alignment",
            &json!({"family": "RF99999", "fmt": "fasta"}),
        )
        .await
        .unwrap();
    let cm = bio
        .call("get_covariance_model", &json!({"family": "RF99999"}))
        .await
        .unwrap();
    let tree = bio
        .call("get_tree", &json!({"family": "RF99999"}))
        .await
        .unwrap();
    let regions = bio
        .call(
            "get_sequence_regions",
            &json!({"family": "RF99999", "max_regions": 2}),
        )
        .await
        .unwrap();
    let mapping = bio
        .call("get_structure_mapping", &json!({"family": "RF99999"}))
        .await
        .unwrap();
    let acc = bio
        .call("accession_to_id", &json!({"accession": "RF99999"}))
        .await
        .unwrap();
    let id = bio
        .call("id_to_accession", &json!({"family_id": "synthRNA"}))
        .await
        .unwrap();
    let search = bio
        .call(
            "search_sequence",
            &json!({"sequence": "AC GU\nACGU", "max_hits": 10}),
        )
        .await
        .unwrap();
    server.abort();

    assert_eq!(alignment["format"], "fasta");
    assert_eq!(alignment["num_sequences"], 2);
    assert_eq!(alignment["source_url"], RFAM_PUBLIC);
    assert!(alignment["alignment"].as_str().unwrap().starts_with('>'));
    assert_eq!(cm["header"]["CLEN"], 8);
    assert!(cm["cm"].as_str().unwrap().contains("INFERNAL"));
    assert_eq!(tree["num_leaf_labels"], 2);
    assert_eq!(regions["declared_count"], 3);
    assert_eq!(regions["num_regions"], 3);
    assert_eq!(regions["returned"], 2);
    assert_eq!(regions["truncated"], true);
    assert_eq!(regions["regions"][0]["sequence_accession"], "SYNTH000001");
    assert_eq!(mapping["pdb_ids"], json!(["1aaa", "9zzz"]));
    assert_eq!(mapping["mapping"][0]["pdb_id"], "1aaa");
    assert_eq!(
        mapping["mapping"][0]["pdb_url"],
        "https://www.rcsb.org/structure/1AAA"
    );
    assert_eq!(acc["rfam_id"], "synthRNA");
    assert_eq!(id["accession"], "RF99999");
    assert_eq!(id["family_url"], "https://rfam.org/family/RF99999");
    assert_eq!(search["num_hits"], 1);
    assert_eq!(search["hits"][0]["acc"], "RF99999");
    assert_eq!(
        search["hits"][0]["family_url"],
        "https://rfam.org/family/RF99999"
    );
    assert!(search["hits"][0].get("alignment").is_none());
    assert_eq!(search["search_sequence"], "ACGUACGU");
    assert_eq!(search["source"], "Rfam");
}

#[tokio::test]
async fn omits_alignment_text_over_max_bytes_without_dropping_metadata() {
    let app = Router::new().route(
        "/family/{family}/alignment",
        get(|| async { format!("{}#=GF CC {}\n", stockholm(), "N".repeat(80)) }),
    );
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "get_seed_alignment",
            &json!({"family": "RF99999", "max_bytes": 20}),
        )
        .await
        .unwrap();
    server.abort();
    assert!(result.get("alignment").is_none());
    assert!(result["alignment_omitted"]
        .as_str()
        .unwrap()
        .contains("max_bytes=20"));
    assert_eq!(result["num_sequences"], 2);
    assert!(result["size_bytes"].as_u64().unwrap() > 20);
}

#[tokio::test]
async fn sequence_search_polls_pending_jobs_until_json_arrives() {
    let polls = Arc::new(StdMutex::new(0u32));
    let count = polls.clone();
    let app = Router::new()
        .route(
            "/search/sequence",
            post(|| async {
                axum::Json(json!({
                    "jobId": "pending-job",
                    "resultURL": "/search/job/pending-job"
                }))
            }),
        )
        .route(
            "/search/job/{job}",
            get(move |Path(job): Path<String>| {
                let count = count.clone();
                async move {
                    assert_eq!(job, "pending-job");
                    let mut n = count.lock().unwrap();
                    *n += 1;
                    if *n == 1 {
                        (StatusCode::ACCEPTED, "PEND").into_response()
                    } else {
                        axum::Json(json!({
                            "hits": {
                                "synthRNA": [{"id": "synthRNA", "acc": "RF99999", "start": 1, "end": 8, "score": 10.0, "E": 0.01}]
                            }
                        }))
                        .into_response()
                    }
                }
            }),
        );
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "search_sequence",
            &json!({"sequence": "ACGU", "poll_interval_s": 0.5, "max_wait_s": 10}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["returned"], 1);
    assert!(*polls.lock().unwrap() >= 2);
}

#[tokio::test]
async fn rejects_rate_limits_malformed_json_html_and_search_backend_errors() {
    for (status, body, tool, args, expected) in [
        (
            StatusCode::TOO_MANY_REQUESTS,
            String::from("secret-token"),
            "get_family",
            json!({"family": "RF99999"}),
            "HTTP 429",
        ),
        (
            StatusCode::OK,
            String::from("{not-json"),
            "get_family",
            json!({"family": "RF99999"}),
            "invalid JSON",
        ),
        (
            StatusCode::OK,
            String::from("<!doctype html><html><body>app</body></html>"),
            "get_family",
            json!({"family": "RF99999"}),
            "HTML",
        ),
        (
            StatusCode::NOT_FOUND,
            String::from("missing"),
            "get_family",
            json!({"family": "RF99999"}),
            "HTTP 404",
        ),
        (
            StatusCode::FORBIDDEN,
            String::from("too-big"),
            "get_sequence_regions",
            json!({"family": "RF99999"}),
            "HTTP 403",
        ),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            String::from("Please come back later"),
            "search_sequence",
            json!({"sequence": "ACGU"}),
            "unavailable",
        ),
    ] {
        let family_body = body.clone();
        let regions_body = body.clone();
        let search_body = body.clone();
        let app = Router::new()
            .route(
                "/family/{family}",
                get({
                    let family_body = family_body.clone();
                    move || {
                        let family_body = family_body.clone();
                        async move {
                            (status, [("retry-after", "60")], family_body).into_response()
                        }
                    }
                }),
            )
            .route(
                "/family/{family}/regions",
                get({
                    let regions_body = regions_body.clone();
                    move || {
                        let regions_body = regions_body.clone();
                        async move {
                            (status, [("retry-after", "60")], regions_body).into_response()
                        }
                    }
                }),
            )
            .route(
                "/search/sequence",
                post({
                    let search_body = search_body.clone();
                    move || {
                        let search_body = search_body.clone();
                        async move {
                            (status, [("retry-after", "60")], search_body).into_response()
                        }
                    }
                }),
            );
        let (bio, server) = serve(app).await;
        let error = bio.call(tool, &args).await.unwrap_err().to_string();
        server.abort();
        assert!(
            error.contains(expected),
            "{tool} {error} did not contain {expected}"
        );
        assert!(!error.contains("secret-token"), "{error}");
    }
}

#[tokio::test]
async fn oversized_result_is_rejected_without_treating_empty_as_success() {
    let app = Router::new().route(
        "/family/{family}",
        get(|| async { (StatusCode::OK, " ".repeat(MAX_RESPONSE + 1)).into_response() }),
    );
    let (bio, server) = serve(app).await;
    let error = bio
        .call("get_family", &json!({"family": "RF99999"}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("exceeded 4 MiB"), "{error}");
}

#[tokio::test]
async fn argument_bounds_fail_before_http_and_unknown_tools_are_rejected() {
    let (bio, server) = serve(Router::new()).await;
    for (name, args) in [
        ("get_family", json!({"family": ""})),
        ("get_family", json!({"family": "RF99999", "api_key": "x"})),
        (
            "get_seed_alignment",
            json!({"family": "RF99999", "max_bytes": 0}),
        ),
        ("search_sequence", json!({"sequence": ">header\nACGU"})),
        (
            "search_sequence",
            json!({"sequence": "ACGU", "max_wait_s": 0}),
        ),
        ("accession_to_id", json!({"accession": "tRNA"})),
    ] {
        let error = bio.call(name, &args).await.unwrap_err().to_string();
        assert!(!error.is_empty(), "{name} {args}");
        assert!(
            !error.contains("connection failed"),
            "{name} reached the network: {error}"
        );
    }
    let error = bio
        .call("not_an_rna_tool", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("unknown native biological tool"));
}
