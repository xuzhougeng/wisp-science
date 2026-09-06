use super::*;
use crate::http::{Http, MAX_RESPONSE};
use crate::NativeBio;
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
            ("GWAS_CATALOG_BASE_URL".into(), base.clone()),
            ("EQTL_METADATA_URL".into(), format!("{base}/metadata")),
            ("EQTL_FILES_URL".into(), base.clone()),
            ("EQTL_ENSEMBL_URL".into(), base.clone()),
            ("PHEWEB_FINNGEN_BASE_URL".into(), base.clone()),
            ("PHEWEB_BBJ_BASE_URL".into(), base),
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

fn association_row() -> Value {
    json!({
        "association_id": "SYNTH-1",
        "p_value": 1e-12,
        "pvalue_mantissa": 1,
        "pvalue_exponent": -12,
        "pvalue_description": "",
        "or_per_copy_num": 1.4,
        "beta": "-",
        "ci_lower": 1.1,
        "ci_upper": 1.8,
        "range": "-",
        "risk_frequency": 0.2,
        "snp_effect_allele": ["rs7412-T"],
        "snp_allele": [{"rs_id": "rs7412"}],
        "locations": ["19:44908822"],
        "mapped_genes": ["APOE"],
        "efo_traits": [{"efo_id": "MONDO_0005010", "efo_trait": "synthetic coronary trait", "uri": "http://example.test/MONDO_0005010"}],
        "bg_efo_traits": [],
        "reported_trait": ["synthetic CAD"],
        "multi_snp_haplotype": false,
        "snp_interaction": false,
        "accession_id": "GCST000001",
        "pubmed_id": "1",
        "first_author": "Synthetic"
    })
}

fn gwas_page(embed: &str, rows: Vec<Value>, total: u64) -> Value {
    let size = rows.len();
    json!({
        "_embedded": { (embed): rows },
        "page": {"size": size, "totalElements": total, "totalPages": 1, "number": 0}
    })
}

#[test]
fn catalog_registers_fourteen_human_genetics_tools() {
    let names: Vec<_> = catalog()
        .into_iter()
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        names,
        vec![
            ("human-genetics", "eqtl_associations".into()),
            ("human-genetics", "eqtl_list_datasets".into()),
            ("human-genetics", "gwas_associations_for_gene".into()),
            ("human-genetics", "gwas_associations_for_trait".into()),
            ("human-genetics", "gwas_associations_for_variant".into()),
            ("human-genetics", "gwas_get_study".into()),
            ("human-genetics", "gwas_get_variant".into()),
            ("human-genetics", "gwas_search_studies".into()),
            ("human-genetics", "gwas_search_traits".into()),
            ("human-genetics", "phewas_finngen_gene".into()),
            ("human-genetics", "phewas_instances".into()),
            ("human-genetics", "phewas_list_phenotypes".into()),
            ("human-genetics", "phewas_search_phenotypes".into()),
            ("human-genetics", "phewas_variant".into()),
        ]
    );
    assert!(crate::contains_tool("gwas_associations_for_variant"));
    assert_eq!(
        crate::domain_for_tool("eqtl_list_datasets"),
        Some("human-genetics")
    );
    assert!(crate::package_selects(
        "mcp_human_genetics",
        "human-genetics"
    ));
    assert!(crate::selected_by_package("mcp_human_genetics"));
}

#[test]
fn rejects_unbounded_or_malformed_identifiers() {
    assert!(require_rs_id(" ").is_err());
    assert!(require_rs_id("chr19:1").is_err());
    assert!(require_rs_id("rs7412/secret").is_err());
    assert_eq!(require_rs_id("RS7412").unwrap(), "rs7412");
    assert!(require_gene_symbol("PCSK9/../APOE").is_err());
    assert!(require_efo_id("not-an-id").is_err());
    assert_eq!(require_efo_id("mondo_0005010").unwrap(), "MONDO_0005010");
    assert!(require_gcst("GCST").is_err());
    assert_eq!(require_gcst("gcst000001").unwrap(), "GCST000001");
    assert!(require_qtd("QTS000001").is_err());
    assert_eq!(require_qtd("qtd000266").unwrap(), "QTD000266");
    assert!(require_ensg("ENSG00000130203.1").is_err());
    assert!(require_pubmed_id("0").is_err());
    assert!(require_eqtl_pos("chr19:1-2").is_err());
    assert_eq!(
        require_eqtl_pos("19:44900000-44920000").unwrap(),
        "19:44900000-44920000"
    );
    assert!(require_eqtl_variant("19_44908822_C_T").is_err());
    assert!(bound_records(0, MAX_GWAS, "max_records").is_err());
    assert!(bound_records(501, MAX_GWAS, "max_records").is_err());
    assert!(serde_json::from_value::<gwas::AssociationsForVariant>(
        json!({"rs_id": "rs7412", "api_key": "secret"})
    )
    .is_err());
}

#[test]
fn gwas_flatten_keeps_study_urls_and_nulls_placeholder_stats() {
    let row = gwas::flatten_association(&association_row());
    assert_eq!(row["or_value"], 1.4);
    assert!(row["beta"].is_null());
    assert!(row["range"].is_null());
    assert_eq!(row["snp_effect_alleles"], json!(["rs7412-T"]));
    assert_eq!(row["rs_ids"], json!(["rs7412"]));
    assert_eq!(
        row["source_url"],
        "https://www.ebi.ac.uk/gwas/studies/GCST000001"
    );
    let snp = gwas::flatten_snp(&json!({
        "rs_id": "rs7412",
        "merged": 0,
        "functional_class": "intron_variant",
        "most_severe_consequence": "intron_variant",
        "alleles": "C/T (forward)",
        "mapped_genes": ["APOE"],
        "locations": [{"chromosome_name": "19", "chromosome_position": 44908822, "region": {"name": "19q13.32"}}],
        "last_update_date": "2020-01-01"
    }));
    assert_eq!(snp["locations"][0]["chromosome"], "19");
    assert_eq!(
        snp["source_url"],
        "https://www.ebi.ac.uk/gwas/variants/rs7412"
    );
}

#[test]
fn pheweb_normalizes_variant_ids_and_ranks_finngen_mlogp() {
    assert_eq!(
        pheweb::normalize_variant_id("chr19:44908822:C:T").unwrap(),
        "19-44908822-C-T"
    );
    assert_eq!(
        pheweb::normalize_variant_id("19_44908822_C_T").unwrap(),
        "19-44908822-C-T"
    );
    assert!(pheweb::normalize_variant_id("19-notpos-C-T").is_err());
    let mut rows = vec![
        json!({"phenocode": "weak", "pval": 0.5}),
        json!({"phenocode": "strong_mlogp", "pval": 5e-324, "mlogp": 400.0}),
        json!({"phenocode": "missing"}),
        json!({"phenocode": "underflow", "pval": 0.0}),
    ];
    rows.sort_by_key(pheweb::phewas_rank);
    assert_eq!(rows[0]["phenocode"], "underflow");
    assert_eq!(rows[1]["phenocode"], "strong_mlogp");
    assert_eq!(rows[2]["phenocode"], "weak");
    assert_eq!(rows[3]["phenocode"], "missing");
}

#[tokio::test]
async fn gwas_tools_dispatch_through_native_bio_call() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = captured.clone();
    let app = Router::new()
        .route(
            "/associations",
            get({
                let seen = seen.clone();
                move |uri: Uri| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(uri.to_string());
                        axum::Json(gwas_page("associations", vec![association_row()], 3))
                    }
                }
            }),
        )
        .route(
            "/studies",
            get({
                let seen = seen.clone();
                move |uri: Uri| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(uri.to_string());
                        axum::Json(gwas_page(
                            "studies",
                            vec![json!({
                                "accession_id": "GCST000001",
                                "disease_trait": "synthetic CAD",
                                "efo_traits": [{"efo_id": "MONDO_0005010", "efo_trait": "synthetic coronary trait"}],
                                "pubmed_id": "1",
                                "full_summary_stats_available": false
                            })],
                            1,
                        ))
                    }
                }
            }),
        )
        .route(
            "/studies/{id}",
            get({
                let seen = seen.clone();
                move |Path(id): Path<String>| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(format!("study/{id}"));
                        if id == "GCST999999" {
                            StatusCode::NOT_FOUND.into_response()
                        } else {
                            axum::Json(json!({
                                "accession_id": id,
                                "disease_trait": "synthetic CAD",
                                "efo_traits": []
                            }))
                            .into_response()
                        }
                    }
                }
            }),
        )
        .route(
            "/efo-traits",
            get({
                let seen = seen.clone();
                move |uri: Uri| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(uri.to_string());
                        axum::Json(gwas_page(
                            "efo_traits",
                            vec![
                                json!({"efo_id": "EFO_0004723", "efo_trait": "synthetic calcification", "uri": "http://example.test/EFO_0004723"}),
                                json!({"efo_id": "MONDO_0005010", "efo_trait": "synthetic coronary trait", "uri": "http://example.test/MONDO_0005010"}),
                            ],
                            2,
                        ))
                    }
                }
            }),
        )
        .route(
            "/single-nucleotide-polymorphisms/{id}",
            get({
                let seen = seen.clone();
                move |Path(id): Path<String>| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(format!("snp/{id}"));
                        axum::Json(json!({
                            "rs_id": id,
                            "merged": 0,
                            "mapped_genes": ["APOE"],
                            "locations": [{"chromosome_name": "19", "chromosome_position": 44908822, "region": {"name": "19q13.32"}}]
                        }))
                        .into_response()
                    }
                }
            }),
        );
    let (bio, server) = serve(app).await;
    let variant = bio
        .call(
            "gwas_associations_for_variant",
            &json!({"rs_id": "RS7412", "max_records": 1}),
        )
        .await
        .unwrap();
    let gene = bio
        .call(
            "gwas_associations_for_gene",
            &json!({"gene_symbol": "PCSK9", "max_records": 1}),
        )
        .await
        .unwrap();
    let trait_hits = bio
        .call(
            "gwas_associations_for_trait",
            &json!({"efo_id": "MONDO_0005010", "max_records": 1}),
        )
        .await
        .unwrap();
    let traits = bio
        .call("gwas_search_traits", &json!({"query": "coronary"}))
        .await
        .unwrap();
    let studies = bio
        .call(
            "gwas_search_studies",
            &json!({"pubmed_id": "1", "max_records": 10}),
        )
        .await
        .unwrap();
    let study = bio
        .call("gwas_get_study", &json!({"accession_id": "GCST000001"}))
        .await
        .unwrap();
    let missing = bio
        .call("gwas_get_study", &json!({"accession_id": "GCST999999"}))
        .await
        .unwrap();
    let snp = bio
        .call("gwas_get_variant", &json!({"rs_id": "rs7412"}))
        .await
        .unwrap();
    server.abort();
    let traffic = captured.lock().unwrap().join("\n");
    assert!(traffic.contains("rs_id=rs7412"), "{traffic}");
    assert!(traffic.contains("sort=p_value"), "{traffic}");
    assert!(traffic.contains("direction=asc"), "{traffic}");
    assert!(traffic.contains("mapped_gene=PCSK9"), "{traffic}");
    assert!(traffic.contains("efo_id=MONDO_0005010"), "{traffic}");
    assert!(traffic.contains("trait=coronary"), "{traffic}");
    assert!(traffic.contains("pubmed_id=1"), "{traffic}");
    assert_eq!(variant["source"], "NHGRI-EBI GWAS Catalog");
    assert_eq!(variant["source_url"], GWAS_SITE);
    assert_eq!(variant["api_total"], 3);
    assert_eq!(variant["returned"], 1);
    assert_eq!(variant["truncated"], true);
    assert_eq!(
        variant["associations"][0]["source_url"],
        "https://www.ebi.ac.uk/gwas/studies/GCST000001"
    );
    assert_eq!(gene["gene_symbol"], "PCSK9");
    assert_eq!(trait_hits["efo_id"], "MONDO_0005010");
    assert_eq!(traits["efo_traits"][0]["efo_id"], "EFO_0004723");
    assert_eq!(studies["filters"]["pubmed_id"], "1");
    assert_eq!(study["found"], true);
    assert_eq!(missing["found"], false);
    assert!(missing["study"].is_null());
    assert_eq!(snp["found"], true);
    assert_eq!(
        snp["variant"]["source_url"],
        "https://www.ebi.ac.uk/gwas/variants/rs7412"
    );
}

#[tokio::test]
async fn eqtl_tools_read_metadata_and_only_indexed_ranges() {
    use noodles_core::Position;
    use noodles_csi::binning_index::index::reference_sequence::bin::Chunk;
    use std::io::Write;
    let mut data = noodles_bgzf::io::Writer::new(Vec::new());
    writeln!(
        data,
        "molecular_trait_id\tchromosome\tposition\tref\talt\tvariant\tpvalue\tbeta\tgene_id\trsid"
    )
    .unwrap();
    let mut index = noodles_tabix::index::Indexer::default();
    for i in 0..3 {
        let pos = 44908822 + i;
        let start = data.virtual_position();
        writeln!(data, "ENSG00000130203\t19\t{pos}\tC\tT\tchr19_{pos}_C_T\t0.000001\t0.4\tENSG00000130203\trs{}", 7412 + i).unwrap();
        index
            .add_record(
                "19",
                Position::try_from(pos).unwrap(),
                Position::try_from(pos).unwrap(),
                Chunk::new(start, data.virtual_position()),
            )
            .unwrap();
    }
    let data = data.finish().unwrap();
    let mut index_bytes = Vec::new();
    {
        let mut writer = noodles_tabix::io::Writer::new(&mut index_bytes);
        writer.write_index(&index.build()).unwrap();
        writer.try_finish().unwrap();
    }
    let captures = Arc::new(StdMutex::new(Vec::new()));
    let seen = captures.clone();
    let app = Router::new()
        .route("/metadata", get(|| async {
            "study_id\tdataset_id\tstudy_label\ttissue_label\tquant_method\tsample_size\tftp_path\nQTS000001\tQTD000266\tGTEx\tliver\tge\t208\tftp://ftp.ebi.ac.uk/pub/databases/spot/eQTL/sumstats/QTS000001/QTD000266/data.tsv.gz\nQTS000001\tQTD000267\tGTEx\tbrain\tge\t99\tftp://ftp.ebi.ac.uk/pub/databases/spot/eQTL/sumstats/QTS000001/QTD000267/data.tsv.gz\n"
        }))
        .route("/QTS000001/QTD000266/data.tsv.gz.tbi", get(move || { let bytes = index_bytes.clone(); async move { bytes } }))
        .route("/QTS000001/QTD000266/data.tsv.gz", get(move |headers: axum::http::HeaderMap| {
            let data = data.clone(); let seen = seen.clone();
            async move {
                let range = headers["range"].to_str().unwrap();
                seen.lock().unwrap().push(range.to_string());
                let (start, end) = range.strip_prefix("bytes=").unwrap().split_once('-').unwrap();
                let start = start.parse::<usize>().unwrap(); let end = end.parse::<usize>().unwrap().min(data.len()-1);
                (StatusCode::PARTIAL_CONTENT, [("content-range", format!("bytes {start}-{end}/{}", data.len()))], data[start..=end].to_vec())
            }
        }));
    let (bio, server) = serve(app).await;
    let datasets = bio
        .call(
            "eqtl_list_datasets",
            &json!({"study_label":"GTEx","quant_method":"ge","max_records":1}),
        )
        .await
        .unwrap();
    assert_eq!(datasets["returned"], 1);
    assert_eq!(datasets["truncated"], true);
    assert_eq!(datasets["datasets"][0]["dataset_id"], "QTD000266");
    let hits = bio.call("eqtl_associations", &json!({"dataset_id":"QTD000266","pos":"19:44908822-44908824","gene_id":"ENSG00000130203","nlog10p_min":5.0,"max_records":2})).await.unwrap();
    assert_eq!(hits["returned"], 2);
    assert_eq!(hits["truncated"], true);
    assert_eq!(hits["associations"][0]["gene_id"], "ENSG00000130203");
    let empty = bio
        .call(
            "eqtl_associations",
            &json!({"dataset_id":"QTD000266","pos":"19:44908822-44908824","rsid":"rs000000"}),
        )
        .await
        .unwrap();
    assert_eq!(empty["returned"], 0);
    assert_eq!(empty["truncated"], false);
    assert!(!captures.lock().unwrap().is_empty());
    server.abort();
}

#[tokio::test]
async fn phewas_tools_dispatch_and_keep_instance_source_urls() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = captured.clone();
    let app = Router::new()
        .route(
            "/api/variant/{id}",
            get({
                let seen = seen.clone();
                move |Path(id): Path<String>| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(format!("variant/{id}"));
                        axum::Json(json!({
                            "variant": {
                                "chr": 19,
                                "pos": 44908822,
                                "ref": "C",
                                "alt": "T",
                                "annotation": {
                                    "rsids": "rs7412",
                                    "gnomad": {"AF": 0.07, "AF_fin": 0.12, "ignored": 1}
                                }
                            },
                            "results": [
                                {"phenocode": "WEAK", "phenostring": "weak", "pval": 0.4, "mlogp": 0.4, "n_case": 10},
                                {"phenocode": "T2D", "phenostring": "synthetic diabetes", "pval": 1e-20, "mlogp": 20.0, "beta": 0.3, "n_case": 100}
                            ]
                        }))
                    }
                }
            }),
        )
        .route(
            "/api/gene_phenos/{gene}",
            get({
                let seen = seen.clone();
                move |Path(gene): Path<String>| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(format!("gene/{gene}"));
                        axum::Json(json!({
                            "phenotypes": [{
                                "assoc": {"phenocode": "T2D", "pval": 1e-8, "mlogp": 8.0},
                                "variant": {"chr": "1", "pos": 55505647, "ref": "G", "alt": "T", "varid": "1:55505647:G:T", "annotation": {"rsids": "rs11591147"}}
                            }]
                        }))
                    }
                }
            }),
        )
        .route(
            "/api/phenos",
            get({
                let seen = seen.clone();
                move || {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push("phenos".into());
                        axum::Json(json!([
                            {"phenocode": "T2D", "phenostring": "synthetic diabetes", "category": "endocrine", "num_cases": 100, "num_controls": 200, "num_gw_significant": 3},
                            {"phenocode": "ASTHMA", "phenostring": "synthetic asthma", "category": "respiratory", "num_cases": 50, "num_controls": 200, "num_gw_significant": 1}
                        ]))
                    }
                }
            }),
        )
        .route(
            "/api/autocomplete",
            get({
                let seen = seen.clone();
                move |Query(params): Query<HashMap<String, String>>| {
                    let seen = seen.clone();
                    async move {
                        seen.lock()
                            .unwrap()
                            .push(format!("autocomplete {}", params.get("query").cloned().unwrap_or_default()));
                        axum::Json(json!([
                            {"display": "synthetic diabetes", "pheno": "T2D", "url": "/pheno/T2D"}
                        ]))
                    }
                }
            }),
        );
    let (bio, server) = serve(app).await;
    let instances = bio.call("phewas_instances", &json!({})).await.unwrap();
    let variant = bio
        .call(
            "phewas_variant",
            &json!({"instance": "finngen", "variant": "chr19:44908822:C:T", "max_phenos": 1}),
        )
        .await
        .unwrap();
    let gene = bio
        .call(
            "phewas_finngen_gene",
            &json!({"gene_symbol": "PCSK9", "max_phenos": 10}),
        )
        .await
        .unwrap();
    let phenos = bio
        .call(
            "phewas_list_phenotypes",
            &json!({"instance": "finngen", "max_records": 1}),
        )
        .await
        .unwrap();
    let search = bio
        .call(
            "phewas_search_phenotypes",
            &json!({"query": "diabetes", "instance": "bbj"}),
        )
        .await
        .unwrap();
    let bbj_list = bio
        .call("phewas_list_phenotypes", &json!({"instance": "bbj"}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    let traffic = captured.lock().unwrap().join("\n");
    assert!(traffic.contains("variant/19-44908822-C-T"), "{traffic}");
    assert!(traffic.contains("gene/PCSK9"), "{traffic}");
    assert!(traffic.contains("autocomplete diabetes"), "{traffic}");
    assert_eq!(instances["instances"]["finngen"]["genome_build"], "GRCh38");
    assert_eq!(
        instances["instances"]["bbj"]["base_url"],
        "https://pheweb.jp"
    );
    assert_eq!(variant["instance"], "finngen");
    assert_eq!(variant["genome_build"], "GRCh38");
    assert_eq!(
        variant["source_url"],
        "https://r12.finngen.fi/variant/19-44908822-C-T"
    );
    assert_eq!(variant["returned"], 1);
    assert_eq!(variant["truncated"], true);
    assert_eq!(variant["phenotypes"][0]["phenocode"], "T2D");
    assert_eq!(variant["variant_meta"]["chrom"], "19");
    assert_eq!(variant["variant_meta"]["gnomad"]["AF"], 0.07);
    assert!(variant["variant_meta"]["gnomad"].get("ignored").is_none());
    assert_eq!(gene["phenotypes"][0]["variant"]["rsids"], "rs11591147");
    assert_eq!(phenos["phenotypes"][0]["phenocode"], "ASTHMA");
    assert_eq!(phenos["truncated"], true);
    assert_eq!(search["instance"], "bbj");
    assert_eq!(search["matches"][0]["phenocode"], "T2D");
    assert!(bbj_list.contains("no phenotypes endpoint"), "{bbj_list}");
}

#[tokio::test]
async fn rejects_rate_limits_malformed_json_and_does_not_echo_secrets() {
    for (status, body, expected) in [
        (
            StatusCode::TOO_MANY_REQUESTS,
            String::from("secret-token"),
            "HTTP 429",
        ),
        (StatusCode::OK, String::from("{not-json"), "invalid JSON"),
    ] {
        let app = Router::new().route(
            "/associations",
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
            .call("gwas_associations_for_variant", &json!({"rs_id": "rs7412"}))
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
        "/associations",
        get(|| async { (StatusCode::OK, " ".repeat(MAX_RESPONSE + 1)).into_response() }),
    );
    let (bio, server) = serve(app).await;
    let error = bio
        .call("gwas_associations_for_variant", &json!({"rs_id": "rs7412"}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("exceeded 4 MiB"), "{error}");
}

#[tokio::test]
async fn gwas_omitted_total_and_empty_promised_page_are_errors() {
    let app = Router::new().route(
        "/associations",
        get(|| async { axum::Json(json!({"_embedded": {"associations": []}})) }),
    );
    let (bio, server) = serve(app).await;
    let error = bio
        .call("gwas_associations_for_variant", &json!({"rs_id": "rs7412"}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("totalElements"), "{error}");

    let app = Router::new().route(
        "/associations",
        get(|| async { axum::Json(gwas_page("associations", vec![], 4)) }),
    );
    let (bio, server) = serve(app).await;
    let error = bio
        .call("gwas_associations_for_variant", &json!({"rs_id": "rs7412"}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("page was empty"), "{error}");
}

#[tokio::test]
async fn unknown_tool_name_and_unfiltered_scans_are_rejected() {
    let (bio, server) = serve(Router::new()).await;
    let unknown = call(&bio, "gwas_not_a_tool", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    let unfiltered = bio
        .call("eqtl_associations", &json!({"dataset_id": "QTD000266"}))
        .await
        .unwrap_err()
        .to_string();
    let both_traits = bio
        .call(
            "gwas_associations_for_trait",
            &json!({"efo_id": "MONDO_0005010", "efo_trait": "coronary artery disorder"}),
        )
        .await
        .unwrap_err()
        .to_string();
    let no_study_filter = bio
        .call("gwas_search_studies", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(unknown.contains("unknown native biological tool"));
    assert!(unfiltered.contains("gene_id / rsid / variant / pos"));
    assert!(both_traits.contains("exactly one"));
    assert!(no_study_filter.contains("at least one"));
}
