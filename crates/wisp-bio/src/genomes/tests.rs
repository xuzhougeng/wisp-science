use super::*;
use crate::http::{Http, MAX_RESPONSE};
use axum::{
    extract::{Path, Query},
    http::{StatusCode, Uri},
    response::IntoResponse,
    routing::get,
    Router,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

fn test_bio(base: &str) -> NativeBio {
    let base = base.trim_end_matches('/').to_string();
    NativeBio::test_client(
        &[
            ("ENSEMBL_BASE_URL".into(), base.clone()),
            ("UCSC_BASE_URL".into(), base),
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

fn gene_record() -> Value {
    json!({
        "id": "ENSG00000000001",
        "display_name": "SYNTH1",
        "description": "synthetic lookup fixture",
        "biotype": "protein_coding",
        "object_type": "Gene",
        "seq_region_name": "1",
        "start": 100,
        "end": 200,
        "strand": 1,
        "assembly_name": "GRCh38",
        "canonical_transcript": "ENST00000000001.1",
        "species": "homo_sapiens",
        "version": 1
    })
}

#[test]
fn catalog_registers_eleven_read_only_genomes_tools() {
    let names: Vec<_> = catalog()
        .into_iter()
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        names,
        vec![
            ("genomes", "ensembl_homology".into()),
            ("genomes", "ensembl_lookup".into()),
            ("genomes", "ensembl_overlap_region".into()),
            ("genomes", "ensembl_sequence".into()),
            ("genomes", "ensembl_vep_variant".into()),
            ("genomes", "ensembl_xrefs".into()),
            ("genomes", "ucsc_chrom_sizes".into()),
            ("genomes", "ucsc_conservation".into()),
            ("genomes", "ucsc_list_tracks".into()),
            ("genomes", "ucsc_tfbs_clusters".into()),
            ("genomes", "ucsc_track_data".into()),
        ]
    );
    assert!(crate::contains_tool("ensembl_lookup"));
    assert_eq!(crate::domain_for_tool("ucsc_track_data"), Some("genomes"));
    assert!(crate::package_selects("mcp_genomes", "genomes"));
    assert!(crate::selected_by_package("mcp_genomes"));
}

#[test]
fn rejects_unbounded_or_unknown_arguments() {
    assert!(
        serde_json::from_value::<LookupArgs>(json!({"query": "SYNTH1", "api_key": "secret"}))
            .is_err()
    );
    assert!(serde_json::from_value::<VepArgs>(json!({"variant_id": "rs1", "token": "x"})).is_err());
    assert!(ident("", "query", MAX_ID, &[]).is_err());
    assert!(ident(&"x".repeat(MAX_ID + 1), "query", MAX_ID, &[]).is_err());
    assert!(ident("SYNTH/1", "query", MAX_ID, &['-']).is_err());
    assert!(region_token("no-colon").is_err());
    assert!(region_token("1:1-2/extra").is_err());
    assert!(require_cap(0, "max_rows", MAX_ROWS).is_err());
    assert!(require_cap(MAX_ROWS + 1, "max_rows", MAX_ROWS).is_err());
    assert!(ucsc_interval(10, 10).is_err());
    assert!(ucsc_interval(-1, 10).is_err());
    assert!(tfbs_track("mm39").is_err());
    assert_eq!(tfbs_track("hg38").unwrap(), "encRegTfbsClustered");
    assert_eq!(tfbs_track("hg19").unwrap(), "wgEncodeRegTfbsClusteredV3");
}

#[test]
fn stable_ids_are_routed_separately_from_ens_prefixed_symbols() {
    assert!(looks_like_stable_id("ENSG00000000001"));
    assert!(looks_like_stable_id("ENSG00000000001.14"));
    assert!(looks_like_stable_id("ENSMUSG00000000001"));
    assert!(looks_like_stable_id("ENST00000000001"));
    assert!(looks_like_stable_id("LRG_1"));
    assert!(!looks_like_stable_id("ENSA"));
    assert!(!looks_like_stable_id("ENSAP1"));
    assert!(!looks_like_stable_id("SYNTH1"));
    assert!(protein_seq_id("ENST00000000001"));
    assert!(protein_seq_id("ENSP00000000001"));
    assert!(!protein_seq_id("ENSG00000000001"));
}

#[test]
fn sha256_and_path_encoding_are_stable() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(path_segment("1:100-200"), "1%3A100-200");
    assert!(!path_segment("id/../xrefs").contains('/'));
    assert!(path_segment("id/../xrefs").contains("%2F"));
}

#[test]
fn vep_summary_orders_by_impact_and_reports_truncation() {
    let summarized = summarize_vep(
        &json!({
            "input": "rsSYNTH1",
            "assembly_name": "GRCh38",
            "seq_region_name": "1",
            "start": 150,
            "end": 150,
            "strand": 1,
            "allele_string": "A/T",
            "most_severe_consequence": "stop_gained",
            "transcript_consequences": [
                {"transcript_id": "ENST2", "gene_id": "ENSG1", "gene_symbol": "SYNTH1", "impact": "LOW"},
                {"transcript_id": "ENST1", "gene_id": "ENSG1", "gene_symbol": "SYNTH1", "impact": "HIGH"}
            ],
            "regulatory_feature_consequences": [{}],
            "colocated_variants": [{"id": "rsSYNTH1", "allele_string": "A/T", "clin_sig": ["benign"], "ignored": true}]
        }),
        1,
    );
    assert_eq!(summarized["n_transcript_consequences"], 2);
    assert_eq!(summarized["transcript_consequences_truncated"], true);
    assert_eq!(summarized["transcript_consequences"][0]["impact"], "HIGH");
    assert_eq!(summarized["genes"][0]["worst_impact"], "HIGH");
    assert_eq!(summarized["genes"][0]["n_transcripts"], 2);
    assert_eq!(summarized["n_regulatory_feature_consequences"], 1);
    assert_eq!(summarized["colocated_variants"][0]["id"], "rsSYNTH1");
    assert!(summarized["colocated_variants"][0].get("ignored").is_none());
}

#[test]
fn conservation_clips_to_the_window_and_weights_by_span() {
    let summary = summarize_conservation(
        &[
            json!({"start": 5, "end": 15, "value": 1.0}),
            json!({"start": 15, "end": 30, "value": 3.0}),
        ],
        10,
        20,
        Some(&json!("wig")),
        "phyloP100way",
    )
    .unwrap();
    assert_eq!(summary.covered, 10);
    assert_eq!(summary.coverage_fraction, 1.0);
    assert_eq!(summary.mean, Some(2.0));
    assert_eq!(summary.values[0]["start"], 10);
    assert_eq!(summary.values[0]["end"], 15);
    assert!(summarize_conservation(
        &[json!({"chromStart": 10, "chromEnd": 20, "name": "peak"})],
        10,
        20,
        Some(&json!("bed 6")),
        "knownGene",
    )
    .is_err());
}

#[test]
fn track_rows_use_the_named_list_or_a_single_subtrack_fallback() {
    let named = extract_track_rows(
        &json!({
            "itemsReturned": 1,
            "phyloP100way": [{"start": 1, "end": 2, "value": 0.5}]
        }),
        "phyloP100way",
        "chr1",
    )
    .unwrap();
    assert_eq!(named.len(), 1);
    let by_chrom = extract_track_rows(
        &json!({
            "itemsReturned": 1,
            "knownGene": {"chrZ": [{"name": "a"}], "chr1": [{"name": "skip"}]}
        }),
        "knownGene",
        "chrZ",
    )
    .unwrap();
    assert_eq!(by_chrom[0]["name"], "a");
    let composite = extract_track_rows(
        &json!({
            "itemsReturned": 1,
            "trackType": "wig",
            "subA": [{"start": 1, "end": 2, "value": 1.0}]
        }),
        "parentTrack",
        "chr1",
    )
    .unwrap();
    assert_eq!(composite.len(), 1);
    assert!(extract_track_rows(
        &json!({
            "itemsReturned": 2,
            "trackType": "bed",
            "a": [1],
            "b": [2]
        }),
        "missing",
        "chr1",
    )
    .is_err());
}

#[tokio::test]
async fn lookup_routes_ids_and_symbols_and_reports_source_urls() {
    let seen = Arc::new(StdMutex::new(Vec::<String>::new()));
    let traffic = seen.clone();
    let app = Router::new()
        .route(
            "/lookup/id/{id}",
            get({
                let traffic = traffic.clone();
                move |Path(id): Path<String>, Query(query): Query<HashMap<String, String>>, uri: Uri| {
                    let traffic = traffic.clone();
                    async move {
                        assert_eq!(query.get("content-type").map(String::as_str), Some("application/json"));
                        traffic.lock().unwrap().push(format!("id {id} expand={} {uri}", query.get("expand").cloned().unwrap_or_default()));
                        if id == "ENSG00000000001" {
                            axum::Json(gene_record()).into_response()
                        } else {
                            StatusCode::BAD_REQUEST.into_response()
                        }
                    }
                }
            }),
        )
        .route(
            "/lookup/symbol/{species}/{symbol}",
            get({
                let traffic = traffic.clone();
                move |Path((species, symbol)): Path<(String, String)>, uri: Uri| {
                    let traffic = traffic.clone();
                    async move {
                        traffic.lock().unwrap().push(format!("symbol {species}/{symbol} {uri}"));
                        if symbol == "SYNTH1" {
                            axum::Json(gene_record()).into_response()
                        } else if symbol == "ENSA" {
                            axum::Json(json!({"id": "ENSG00000000002", "display_name": "ENSA", "object_type": "Gene", "species": "homo_sapiens"})).into_response()
                        } else {
                            StatusCode::BAD_REQUEST.into_response()
                        }
                    }
                }
            }),
        );
    let (bio, server) = serve(app).await;
    let by_id = bio
        .call(
            "ensembl_lookup",
            &json!({"query": "ENSG00000000001", "expand": true}),
        )
        .await
        .unwrap();
    let by_symbol = bio
        .call("ensembl_lookup", &json!({"query": "SYNTH1"}))
        .await
        .unwrap();
    let ensa = bio
        .call("ensembl_lookup", &json!({"query": "ENSA"}))
        .await
        .unwrap();
    let missing = bio
        .call("ensembl_lookup", &json!({"query": "ENSG00009999999"}))
        .await
        .unwrap();
    server.abort();
    let log = seen.lock().unwrap().join("\n");
    assert!(log.contains("id ENSG00000000001 expand=1"), "{log}");
    assert!(log.contains("symbol homo_sapiens/SYNTH1"), "{log}");
    assert!(log.contains("symbol homo_sapiens/ENSA"), "{log}");
    assert!(!log.contains("/lookup/id/ENSA"), "{log}");
    assert_eq!(by_id["source"], "Ensembl REST");
    assert_eq!(by_id["source_url"], ENSEMBL_REST);
    assert_eq!(by_id["found"], true);
    assert_eq!(
        by_id["url"],
        "https://www.ensembl.org/homo_sapiens/Gene/Summary?g=ENSG00000000001"
    );
    assert_eq!(by_symbol["record"]["display_name"], "SYNTH1");
    assert_eq!(ensa["found"], true);
    assert_eq!(missing["found"], false);
    assert!(missing["record"].is_null());
}

#[tokio::test]
async fn remaining_ensembl_tools_dispatch_through_native_bio_call() {
    let seen = Arc::new(StdMutex::new(Vec::<String>::new()));
    let traffic = seen.clone();
    let capture = move |label: &'static str| {
        let traffic = traffic.clone();
        move |uri: Uri| {
            let traffic = traffic.clone();
            async move {
                assert!(
                    uri.query()
                        .unwrap_or("")
                        .contains("content-type=application%2Fjson"),
                    "{uri}"
                );
                traffic.lock().unwrap().push(format!("{label} {uri}"));
            }
        }
    };
    let app = Router::new()
        .route(
            "/xrefs/id/{id}",
            get({
                let mark = capture("xrefs");
                move |Path(id): Path<String>, Query(query): Query<HashMap<String, String>>, uri: Uri| {
                    let mark = mark.clone();
                    async move {
                        mark(uri).await;
                        if id == "missing" {
                            return StatusCode::BAD_REQUEST.into_response();
                        }
                        let db = query.get("external_db").cloned().unwrap_or_default();
                        axum::Json(json!([
                            {"dbname": "EntrezGene", "primary_id": "2", "display_id": "SYNTH1"},
                            {"dbname": "HGNC", "primary_id": "1", "display_id": "SYNTH1", "keep": db}
                        ]))
                        .into_response()
                    }
                }
            }),
        )
        .route(
            "/vep/{species}/id/{id}",
            get({
                let mark = capture("vep-id");
                move |uri: Uri| {
                    let mark = mark.clone();
                    async move {
                        mark(uri).await;
                        axum::Json(json!([{
                            "input": "rsSYNTH1",
                            "assembly_name": "GRCh38",
                            "most_severe_consequence": "missense_variant",
                            "transcript_consequences": [
                                {"transcript_id": "ENSTB", "gene_id": "ENSG1", "impact": "MODIFIER"},
                                {"transcript_id": "ENSTA", "gene_id": "ENSG1", "impact": "HIGH"}
                            ],
                            "colocated_variants": [{"id": "rsSYNTH1", "start": 150}]
                        }]))
                        .into_response()
                    }
                }
            }),
        )
        .route(
            "/vep/{species}/region/{region}/{allele}",
            get({
                let mark = capture("vep-region");
                move |Path((_, region, allele)): Path<(String, String, String)>, uri: Uri| {
                    let mark = mark.clone();
                    async move {
                        mark(uri).await;
                        axum::Json(json!([{
                            "input": format!("{region}/{allele}"),
                            "most_severe_consequence": "intergenic_variant",
                            "transcript_consequences": []
                        }]))
                        .into_response()
                    }
                }
            }),
        )
        .route(
            "/lookup/symbol/{species}/{symbol}",
            get({
                let mark = capture("lookup-symbol");
                move |uri: Uri| {
                    let mark = mark.clone();
                    async move {
                        mark(uri).await;
                        axum::Json(gene_record()).into_response()
                    }
                }
            }),
        )
        .route(
            "/homology/id/{species}/{id}",
            get({
                let mark = capture("homology");
                move |Query(query): Query<HashMap<String, String>>, uri: Uri| {
                    let mark = mark.clone();
                    async move {
                        mark(uri).await;
                        assert_eq!(query.get("format").map(String::as_str), Some("condensed"));
                        axum::Json(json!({
                            "data": [{
                                "id": "ENSG00000000001",
                                "homologies": [
                                    {"type": "ortholog_one2one", "species": "mus_musculus", "id": "ENSMUSG00000000002", "protein_id": "ENSMUSP00000000002", "taxonomy_level": "Euarchontoglires", "method_link_type": "ENSEMBL_ORTHOLOGUES"},
                                    {"type": "ortholog_one2one", "species": "danio_rerio", "id": "ENSDARG00000000003"}
                                ]
                            }]
                        }))
                        .into_response()
                    }
                }
            }),
        )
        .route(
            "/sequence/id/{id}",
            get({
                let mark = capture("seq-id");
                move |Query(query): Query<HashMap<String, String>>, uri: Uri| {
                    let mark = mark.clone();
                    async move {
                        mark(uri).await;
                        let seq = if query.get("type").map(String::as_str) == Some("protein") {
                            "M".repeat(12)
                        } else {
                            "ACGTACGTACGT".into()
                        };
                        axum::Json(json!({"id": "ENST00000000001", "desc": "synthetic", "molecule": "dna", "seq": seq}))
                            .into_response()
                    }
                }
            }),
        )
        .route(
            "/sequence/region/{species}/{region}",
            get({
                let mark = capture("seq-region");
                move |Path((_, region)): Path<(String, String)>, uri: Uri| {
                    let mark = mark.clone();
                    async move {
                        mark(uri).await;
                        axum::Json(json!({"id": region, "molecule": "dna", "seq": "ACGT"}))
                            .into_response()
                    }
                }
            }),
        )
        .route(
            "/overlap/region/{species}/{region}",
            get({
                let mark = capture("overlap");
                move |Query(query): Query<HashMap<String, String>>, uri: Uri| {
                    let mark = mark.clone();
                    async move {
                        mark(uri).await;
                        assert_eq!(query.get("feature").map(String::as_str), Some("gene"));
                        axum::Json(json!([
                            {"id": "ENSG00000000002", "start": 50, "external_name": "B"},
                            {"id": "ENSG00000000001", "start": 50, "external_name": "A"},
                            {"id": "ENSG00000000003", "start": 80, "external_name": "C"}
                        ]))
                        .into_response()
                    }
                }
            }),
        );
    let (bio, server) = serve(app).await;
    let xrefs = bio
        .call(
            "ensembl_xrefs",
            &json!({"stable_id": "ENSG00000000001", "external_db": "HGNC"}),
        )
        .await
        .unwrap();
    let missing_xrefs = bio
        .call("ensembl_xrefs", &json!({"stable_id": "missing"}))
        .await
        .unwrap();
    let vep = bio
        .call(
            "ensembl_vep_variant",
            &json!({"variant_id": "rsSYNTH1", "max_consequences": 1}),
        )
        .await
        .unwrap();
    let vep_region = bio
        .call(
            "ensembl_vep_variant",
            &json!({"region": "1:150-150", "allele": "T"}),
        )
        .await
        .unwrap();
    let homology = bio
        .call(
            "ensembl_homology",
            &json!({"gene_symbol": "SYNTH1", "max_homologies": 1, "target_taxon": 9443}),
        )
        .await
        .unwrap();
    let seq = bio
        .call(
            "ensembl_sequence",
            &json!({"stable_id": "ENST00000000001", "max_bytes": 4}),
        )
        .await
        .unwrap();
    let seq_region = bio
        .call(
            "ensembl_sequence",
            &json!({"region": "1:100-103", "seq_type": "protein"}),
        )
        .await
        .unwrap();
    let overlap = bio
        .call(
            "ensembl_overlap_region",
            &json!({"region": "1:1-200", "max_features": 2}),
        )
        .await
        .unwrap();
    server.abort();
    let log = seen.lock().unwrap().join("\n");
    assert!(log.contains("xrefs /xrefs/id/ENSG00000000001"), "{log}");
    assert!(log.contains("external_db=HGNC"), "{log}");
    assert!(
        log.contains("vep-id /vep/homo_sapiens/id/rsSYNTH1"),
        "{log}"
    );
    assert!(
        log.contains("vep-region /vep/homo_sapiens/region/1%3A150-150/T"),
        "{log}"
    );
    assert!(
        log.contains("lookup-symbol /lookup/symbol/homo_sapiens/SYNTH1"),
        "{log}"
    );
    assert!(
        log.contains("homology /homology/id/homo_sapiens/ENSG00000000001"),
        "{log}"
    );
    assert!(log.contains("target_taxon=9443"), "{log}");
    assert!(!log.contains("/homology/symbol/"), "{log}");
    assert_eq!(xrefs["n_xrefs"], 2);
    assert_eq!(xrefs["xrefs"][0]["dbname"], "EntrezGene");
    assert_eq!(missing_xrefs["n_xrefs"], 0);
    assert_eq!(vep["n_results"], 1);
    assert_eq!(vep["results"][0]["transcript_consequences_truncated"], true);
    assert_eq!(
        vep["results"][0]["transcript_consequences"][0]["impact"],
        "HIGH"
    );
    assert_eq!(vep_region["query"]["allele"], "T");
    assert_eq!(homology["gene_id"], "ENSG00000000001");
    assert_eq!(homology["n_total"], 2);
    assert_eq!(homology["homologies_truncated"], true);
    assert_eq!(homology["homologies"][0]["species"], "danio_rerio");
    assert_eq!(seq["found"], true);
    assert!(seq.get("seq").is_none(), "{seq}");
    assert!(seq["seq_omitted"].as_str().unwrap().contains("max_bytes=4"));
    assert_eq!(seq["sha256"], sha256_hex(b"ACGTACGTACGT"));
    assert_eq!(seq_region["seq_type"], "genomic");
    assert_eq!(seq_region["seq"], "ACGT");
    assert_eq!(overlap["n_total"], 3);
    assert_eq!(overlap["features_truncated"], true);
    assert_eq!(overlap["features"][0]["id"], "ENSG00000000001");
    assert_eq!(overlap["source_url"], ENSEMBL_REST);
}

#[tokio::test]
async fn ucsc_tools_dispatch_through_native_bio_call() {
    let seen = Arc::new(StdMutex::new(Vec::<String>::new()));
    let traffic = seen.clone();
    let app = Router::new()
        .route(
            "/list/tracks",
            get({
                let traffic = traffic.clone();
                move |Query(query): Query<HashMap<String, String>>, uri: Uri| {
                    let traffic = traffic.clone();
                    async move {
                        traffic.lock().unwrap().push(format!("tracks {uri}"));
                        let genome = query.get("genome").cloned().unwrap_or_else(|| "hg38".into());
                        axum::Json(json!({
                            genome: {
                                "phyloP100way": {"shortLabel": "phyloP", "longLabel": "100-way conservation", "type": "wig", "group": "compGeno", "parent": "cons100way"},
                                "knownGene": {"shortLabel": "GENCODE", "longLabel": "gene models", "type": "genePred", "group": "genes"},
                                "encRegTfbsClustered": {"shortLabel": "TFBS", "longLabel": "ENCODE TF clusters", "type": "factorSource"}
                            }
                        }))
                        .into_response()
                    }
                }
            }),
        )
        .route(
            "/list/chromosomes",
            get({
                let traffic = traffic.clone();
                move |uri: Uri| {
                    let traffic = traffic.clone();
                    async move {
                        traffic.lock().unwrap().push(format!("chroms {uri}"));
                        axum::Json(json!({
                            "chromCount": 4,
                            "chromosomes": {
                                "chr1": 1000,
                                "chr2": 800,
                                "chrUn_fix": 50,
                                "chrM": 16
                            }
                        }))
                        .into_response()
                    }
                }
            }),
        )
        .route(
            "/getData/track",
            get({
                let traffic = traffic.clone();
                move |Query(query): Query<HashMap<String, String>>, uri: Uri| {
                    let traffic = traffic.clone();
                    async move {
                        traffic.lock().unwrap().push(format!("data {uri}"));
                        let track = query.get("track").cloned().unwrap_or_default();
                        if track == "phyloP100way" {
                            return axum::Json(json!({
                                "trackType": "wig",
                                "itemsReturned": 2,
                                "phyloP100way": [
                                    {"start": 5, "end": 15, "value": 1.0},
                                    {"start": 15, "end": 30, "value": 3.0}
                                ]
                            }))
                            .into_response();
                        }
                        if track == "encRegTfbsClustered" {
                            return axum::Json(json!({
                                "trackType": "factorSource",
                                "itemsReturned": 2,
                                "maxItemsLimit": query.get("maxItemsOutput").map(|n| n == "1").unwrap_or(false),
                                "encRegTfbsClustered": [
                                    {"name": "CTCF", "chrom": "chrZ", "chromStart": 20, "chromEnd": 30, "score": 800, "sourceCount": 12},
                                    {"name": "SYNTH", "chrom": "chrZ", "chromStart": 10, "chromEnd": 18, "score": 400, "sourceCount": 2}
                                ]
                            }))
                            .into_response();
                        }
                        axum::Json(json!({
                            "trackType": "bed 6",
                            "itemsReturned": 1,
                            "maxItemsLimit": true,
                            "dataDownloadUrl": "https://hgdownload.example.test/knownGene.bb",
                            "knownGene": [{"chrom": "chrZ", "chromStart": 10, "chromEnd": 20, "name": "SYNTH1"}]
                        }))
                        .into_response()
                    }
                }
            }),
        );
    let (bio, server) = serve(app).await;
    let tracks = bio
        .call(
            "ucsc_list_tracks",
            &json!({"filter_text": "phylo", "max_tracks": 1}),
        )
        .await
        .unwrap();
    let chroms = bio
        .call(
            "ucsc_chrom_sizes",
            &json!({"filter_text": "chr", "max_chroms": 2}),
        )
        .await
        .unwrap();
    let conservation = bio
        .call(
            "ucsc_conservation",
            &json!({"chrom": "chrZ", "start": 10, "end": 20, "include_values": true}),
        )
        .await
        .unwrap();
    let tfbs = bio
        .call(
            "ucsc_tfbs_clusters",
            &json!({"chrom": "chrZ", "start": 0, "end": 50, "max_rows": 10}),
        )
        .await
        .unwrap();
    let data = bio
        .call(
            "ucsc_track_data",
            &json!({"track": "knownGene", "chrom": "chrZ", "start": 0, "end": 50, "max_rows": 5}),
        )
        .await
        .unwrap();
    server.abort();
    let log = seen.lock().unwrap().join("\n");
    assert!(log.contains("/list/tracks"), "{log}");
    assert!(log.contains("trackLeavesOnly=1"), "{log}");
    assert!(log.contains("/list/chromosomes"), "{log}");
    assert!(log.contains("maxItemsOutput=5"), "{log}");
    assert_eq!(tracks["n_total"], 1);
    assert_eq!(tracks["tracks"][0]["track"], "phyloP100way");
    assert_eq!(tracks["source_url"], UCSC_API);
    assert_eq!(chroms["chrom_count"], 4);
    assert_eq!(chroms["n_total"], 4);
    assert_eq!(chroms["chroms_truncated"], true);
    assert_eq!(chroms["chromosomes"][0]["name"], "chr1");
    assert_eq!(conservation["n_bases_covered"], 10);
    assert_eq!(conservation["mean"], 2.0);
    assert_eq!(conservation["values"][0]["start"], 10);
    assert_eq!(tfbs["track"], "encRegTfbsClustered");
    assert_eq!(tfbs["n_factors"], 2);
    assert_eq!(tfbs["clusters"][0]["name"], "SYNTH");
    assert_eq!(data["truncated"], true);
    assert_eq!(
        data["data_download_url"],
        "https://hgdownload.example.test/knownGene.bb"
    );
    assert_eq!(
        data["browser_url"],
        "https://genome.ucsc.edu/cgi-bin/hgTracks?db=hg38&position=chrZ:1-50"
    );
}

#[tokio::test]
async fn conservation_refuses_truncated_or_non_score_tracks() {
    let app = Router::new().route(
        "/getData/track",
        get(|Query(query): Query<HashMap<String, String>>| async move {
            if query.get("track").map(String::as_str) == Some("knownGene") {
                axum::Json(json!({
                    "trackType": "genePred",
                    "itemsReturned": 1,
                    "knownGene": [{"chromStart": 1, "chromEnd": 2, "name": "x"}]
                }))
            } else {
                axum::Json(json!({
                    "trackType": "wig",
                    "itemsReturned": 1,
                    "maxItemsLimit": true,
                    "phyloP100way": [{"start": 0, "end": 1, "value": 1.0}]
                }))
            }
        }),
    );
    let (bio, server) = serve(app).await;
    let truncated = bio
        .call(
            "ucsc_conservation",
            &json!({"chrom": "chr1", "start": 0, "end": 10}),
        )
        .await
        .unwrap_err()
        .to_string();
    let bed = bio
        .call(
            "ucsc_conservation",
            &json!({"chrom": "chr1", "start": 0, "end": 10, "track": "knownGene"}),
        )
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(truncated.contains("truncated"), "{truncated}");
    assert!(bed.contains("per-base values"), "{bed}");
}

#[tokio::test]
async fn rejects_rate_limits_and_malformed_json_without_echoing_secrets() {
    for (status, body, expected) in [
        (
            StatusCode::TOO_MANY_REQUESTS,
            "secret-token".to_string(),
            "HTTP 429",
        ),
        (StatusCode::OK, "{not-json".into(), "invalid JSON"),
    ] {
        let app = Router::new().route(
            "/lookup/id/{id}",
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
            .call("ensembl_lookup", &json!({"query": "ENSG00000000001"}))
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
async fn ucsc_rate_limit_and_invalid_json_are_classified_without_bodies() {
    for (status, body, expected) in [
        (
            StatusCode::TOO_MANY_REQUESTS,
            "secret-token".to_string(),
            "HTTP 429",
        ),
        (StatusCode::OK, "{not-json".into(), "invalid JSON"),
    ] {
        let app = Router::new().route(
            "/list/chromosomes",
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
            .call("ucsc_chrom_sizes", &json!({}))
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
async fn oversized_listing_is_rejected() {
    let app = Router::new().route(
        "/list/tracks",
        get(|| async { (StatusCode::OK, " ".repeat(MAX_RESPONSE + 1)).into_response() }),
    );
    let (bio, server) = serve(app).await;
    let error = bio
        .call("ucsc_list_tracks", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("exceeded 4 MiB"), "{error}");
}

#[tokio::test]
async fn xor_routes_and_unknown_tools_are_rejected() {
    let (bio, server) = serve(Router::new()).await;
    for (name, args, expected) in [
        ("ensembl_vep_variant", json!({}), "exactly one"),
        (
            "ensembl_vep_variant",
            json!({"variant_id": "rs1", "region": "1:1-1", "allele": "T"}),
            "exactly one",
        ),
        (
            "ensembl_sequence",
            json!({"stable_id": "ENSG00000000001", "region": "1:1-2"}),
            "exactly one",
        ),
        ("ensembl_homology", json!({}), "exactly one"),
        (
            "ensembl_sequence",
            json!({"stable_id": "ENSG00000000001", "seq_type": "protein"}),
            "protein is only valid",
        ),
        (
            "ucsc_conservation",
            json!({"chrom": "chr1", "start": 0, "end": 200000}),
            "100000",
        ),
    ] {
        let error = bio.call(name, &args).await.unwrap_err().to_string();
        assert!(
            error.contains(expected),
            "{name} {error} did not contain {expected}"
        );
    }
    let error = call(&bio, "ensembl_not_a_tool", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("unknown native biological tool"));
}
