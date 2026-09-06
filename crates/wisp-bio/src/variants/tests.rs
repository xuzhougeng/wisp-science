use super::*;
use crate::http::{Http, MAX_RESPONSE};
use crate::NativeBio;
use axum::{
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex as StdMutex};

fn test_bio(base: &str) -> NativeBio {
    let base = base.trim_end_matches('/');
    NativeBio::test_client(
        &[
            ("NCBI_EMAIL".into(), "operator@example.test".into()),
            ("NCBI_API_KEY".into(), "synthetic-key&value".into()),
            ("GNOMAD_API_URL".into(), format!("{base}/api")),
            ("NCBI_EUTILS_URL".into(), format!("{base}/")),
            ("NCBI_VARIATION_URL".into(), format!("{base}/variation/v0")),
            ("CADD_BASE_URL".into(), format!("{base}/cadd")),
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

async fn cadd_serve(app: Router) -> (NativeBio, tokio::task::JoinHandle<()>) {
    serve(Router::new().nest("/cadd", app)).await
}

fn cadd_position_rows() -> Value {
    json!([
        {
            "Alt": "T",
            "Chrom": "4",
            "PHRED": "0.010",
            "Pos": "9001",
            "RawScore": "0.003",
            "Ref": "A"
        },
        {
            "Alt": "C",
            "Chrom": "4",
            "PHRED": "0.850",
            "Pos": "9001",
            "RawScore": "-0.251851",
            "Ref": "A"
        },
        {
            "Alt": "G",
            "Chrom": "4",
            "PHRED": "15.20",
            "Pos": "9001",
            "RawScore": "1.234567",
            "Ref": "A"
        }
    ])
}

fn cadd_two_alts() -> Value {
    json!([
        {
            "Alt": "G",
            "Chrom": "4",
            "PHRED": "15.20",
            "Pos": "9001",
            "RawScore": "1.234567",
            "Ref": "A"
        },
        {
            "Alt": "C",
            "Chrom": "4",
            "PHRED": "0.850",
            "Pos": "9001",
            "RawScore": "-0.251851",
            "Ref": "A"
        }
    ])
}

fn cadd_range_rows() -> Value {
    json!([
        ["Chrom", "Pos", "Ref", "Alt", "RawScore", "PHRED"],
        ["4", "9002", "A", "T", "0.121712", "2.838"],
        ["4", "9001", "A", "G", "1.234567", "15.20"],
        ["4", "9001", "A", "C", "-0.251851", "0.850"]
    ])
}

#[test]
fn catalog_registers_eighteen_variant_tools_including_cadd() {
    let names: Vec<_> = catalog()
        .into_iter()
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        names,
        vec![
            ("variants", "cadd_position_scores".into()),
            ("variants", "cadd_range_scores".into()),
            ("variants", "cadd_variant_score".into()),
            ("variants", "get_variant".into()),
            ("variants", "search_variants".into()),
            ("variants", "gene_variants".into()),
            ("variants", "gene_constraint".into()),
            ("variants", "region_variants".into()),
            ("variants", "liftover_variant".into()),
            ("variants", "clinvar_variants".into()),
            ("variants", "structural_variants".into()),
            ("variants", "get_structural_variant".into()),
            ("variants", "mitochondrial_variants".into()),
            ("variants", "clinvar_search".into()),
            ("variants", "clinvar_get_records".into()),
            ("variants", "clinvar_variant_by_rsid".into()),
            ("variants", "dbsnp_get_rsids".into()),
            ("variants", "dbsnp_search_by_region".into()),
        ]
    );
    assert_eq!(names.len(), 18);
    assert!(crate::contains_tool("get_variant"));
    assert!(crate::contains_tool("cadd_variant_score"));
    assert!(crate::contains_tool("cadd_position_scores"));
    assert!(crate::contains_tool("cadd_range_scores"));
    assert_eq!(crate::domain_for_tool("get_variant"), Some("variants"));
    assert_eq!(
        crate::domain_for_tool("cadd_position_scores"),
        Some("variants")
    );
    assert!(crate::package_selects("mcp_variants", "variants"));
    assert!(crate::selected_by_package("mcp_variants"));
    for name in [
        "cadd_position_scores",
        "cadd_range_scores",
        "cadd_variant_score",
    ] {
        let description = catalog()
            .into_iter()
            .find(|(_, schema)| schema.function.name == name)
            .unwrap()
            .1
            .function
            .description;
        assert!(
            description.contains("non-commercial") && description.contains("experimental"),
            "{name}: {description}"
        );
        assert!(description.contains("PHRED ≥ 20"), "{name}: {description}");
    }
}

#[test]
fn validates_identifiers_regions_and_gene_xor() {
    assert!(require_variant_id("19-44908822-C-T").is_ok());
    assert_eq!(
        require_variant_id("chr19-44908822-c-t").unwrap(),
        "19-44908822-C-T"
    );
    assert!(require_variant_id("not-an-id").is_err());
    assert!(require_variant_id("19-0-C-T").is_err());
    assert_eq!(normalize_chrom("chrX", ChromKind::Nuclear).unwrap(), "X");
    assert_eq!(normalize_chrom("MT", ChromKind::Dbsnp).unwrap(), "MT");
    assert!(normalize_chrom("M", ChromKind::Nuclear).is_err());
    assert!(require_region(10, 9).is_err());
    assert!(require_region(1, REGION_SPAN + 2).is_err());
    assert!(require_region(100, 200).is_ok());
    assert_eq!(require_rsid("RS7412").unwrap(), (7412, "rs7412".into()));
    assert!(require_rsid("rs").is_err());
    assert!(gene_args(&Some("TP53".into()), &None).is_ok());
    assert!(gene_args(&Some("TP53".into()), &Some("ENSG1".into())).is_err());
    assert!(gene_args(&None, &None).is_err());
    assert!(require_dataset("gnomad_r4").is_ok());
    assert!(require_dataset("gnomad_r5").is_err());
    assert_eq!(reference_genome("gnomad_r2_1"), "GRCh37");
    assert_eq!(reference_genome("gnomad_r4"), "GRCh38");
}

#[test]
fn clinvar_gold_stars_follow_official_review_status() {
    assert_eq!(clinvar::gold_stars("practice guideline"), json!(4));
    assert_eq!(clinvar::gold_stars("reviewed by expert panel"), json!(3));
    assert_eq!(
        clinvar::gold_stars("criteria provided, multiple submitters, no conflicts"),
        json!(2)
    );
    assert_eq!(
        clinvar::gold_stars("criteria provided, conflicting classifications"),
        json!(1)
    );
    assert_eq!(
        clinvar::gold_stars("no assertion criteria provided"),
        json!(0)
    );
    assert_eq!(clinvar::gold_stars("mystery status"), Value::Null);
    let record = clinvar::parse_summary(&json!({
        "uid": "45122",
        "accession": "VCV000045122",
        "accession_version": "VCV000045122.3",
        "title": "synthetic variant",
        "obj_type": "single nucleotide variant",
        "protein_change": "R175H",
        "variation_set": [{
            "variant_type": "single nucleotide variant",
            "canonical_spdi": "NC_000017.11:7675088:C:T",
            "variation_xrefs": [{"db_source": "dbSNP", "db_id": "121913343"}],
            "variation_loc": [{
                "status": "current",
                "assembly_name": "GRCh38",
                "chr": "17",
                "start": "7675089",
                "stop": "7675089",
                "ref": "C",
                "alt": "T"
            }],
            "allele_freq_set": [{"source": "gnomAD", "minor_allele": "T", "value": "0.0001"}]
        }],
        "genes": [{"symbol": "TP53", "geneid": "7157", "strand": "-"}],
        "molecular_consequence_list": ["missense variant"],
        "germline_classification": {
            "description": "Pathogenic",
            "review_status": "criteria provided, single submitter",
            "last_evaluated": "2022/10/12 00:00",
            "trait_set": [{"trait_name": "Li-Fraumeni syndrome", "trait_xrefs": [{"db_source": "MedGen", "db_id": "C0085390"}]}]
        },
        "supporting_submissions": {"scv": ["SCV000000001"], "rcv": ["RCV000000001"]}
    }))
    .unwrap();
    assert_eq!(record["variation_id"], 45122);
    assert_eq!(record["rsids"], json!(["rs121913343"]));
    assert_eq!(record["germline_classification"]["gold_stars"], 1);
    assert_eq!(
        record["germline_classification"]["last_evaluated"],
        "2022-10-12"
    );
    assert_eq!(
        record["url"],
        "https://www.ncbi.nlm.nih.gov/clinvar/variation/45122/"
    );
    assert!(serde_json::from_value::<gnomad::GetVariant>(json!({
        "variant_id": "19-44908822-C-T", "api_key": "secret"
    }))
    .is_err());
}

#[test]
fn dbsnp_distill_preserves_merged_live_and_placements() {
    let merged = dbsnp::distill_refsnp(&json!({
        "refsnp_id": "1",
        "merged_snapshot_data": {"merged_into": ["7412"]},
        "citations": [1, 2]
    }))
    .unwrap();
    assert_eq!(merged["status"], "merged");
    assert_eq!(merged["merged_into"], json!(["rs7412"]));
    let live = dbsnp::distill_refsnp(&json!({
        "refsnp_id": "7412",
        "citations": (0..25).collect::<Vec<_>>(),
        "mane_select_ids": ["ENST00000252486.9"],
        "primary_snapshot_data": {
            "variant_type": "snv",
            "placements_with_allele": [{
                "seq_id": "NC_000019.10",
                "is_ptlp": true,
                "placement_annot": {"seq_id_traits_by_assembly": [{
                    "is_chromosome": true, "assembly_name": "GRCh38.p14"
                }]},
                "alleles": [
                    {"allele": {"spdi": {"seq_id": "NC_000019.10", "position": 44908821, "deleted_sequence": "C", "inserted_sequence": "C"}}, "hgvs": "NC_000019.10:g.44908822C"},
                    {"allele": {"spdi": {"seq_id": "NC_000019.10", "position": 44908821, "deleted_sequence": "C", "inserted_sequence": "T"}}, "hgvs": "NC_000019.10:g.44908822C>T"}
                ]
            }],
            "allele_annotations": [
                {},
                {
                    "frequency": [{"study_name": "gnomAD", "study_version": 4, "allele_count": 1, "total_count": 1000}],
                    "clinical": [{"accession_version": "RCV000000001.1", "clinical_significances": ["benign"], "review_status": "criteria provided, single submitter", "disease_names": ["synthetic"]}],
                    "assembly_annotation": [{"genes": [{
                        "locus": "APOE", "id": "348", "name": "apolipoprotein E", "orientation": "plus",
                        "rnas": [{"id": "ENST00000252486.9", "hgvs": "NM_000041.4:c.526C>T", "sequence_ontology": [{"name": "missense_variant"}], "protein": {"variant": {"spdi": {"seq_id": "NP_000032.1", "position": 175, "deleted_sequence": "R", "inserted_sequence": "C"}}}}]
                    }]}]
                }
            ]
        }
    }))
    .unwrap();
    assert_eq!(live["status"], "live");
    assert_eq!(live["citations_truncated"], true);
    assert_eq!(live["placements"][0]["chrom"], "19");
    assert_eq!(live["placements"][0]["position"], 44908822);
    assert_eq!(live["alleles"][0]["allele"], "T");
    assert_eq!(live["alleles"][0]["genes"][0]["symbol"], "APOE");
    assert_eq!(live["url"], "https://www.ncbi.nlm.nih.gov/snp/rs7412");
}

#[tokio::test]
async fn get_variant_posts_graphql_and_reports_source_urls() {
    let captured = Arc::new(StdMutex::new(String::new()));
    let body = captured.clone();
    let app = Router::new().route(
        "/api",
        post(move |incoming: String| {
            *body.lock().unwrap() = incoming;
            async {
                axum::Json(json!({
                    "data": {
                        "variant": {
                            "variant_id": "19-44908822-C-T",
                            "reference_genome": "GRCh38",
                            "chrom": "19", "pos": 44908822, "ref": "C", "alt": "T",
                            "rsids": ["rs7412"],
                            "exome": {"ac": 1, "an": 100, "af": 0.01, "homozygote_count": 0, "hemizygote_count": 0, "filters": ["PASS"]},
                            "genome": null
                        }
                    }
                }))
            }
        }),
    );
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "get_variant",
            &json!({"variant_id": "chr19-44908822-c-t", "dataset": "gnomad_r4"}),
        )
        .await
        .unwrap();
    server.abort();
    let posted = captured.lock().unwrap().clone();
    assert!(
        posted.contains("\"variantId\":\"19-44908822-C-T\""),
        "{posted}"
    );
    assert!(posted.contains("\"dataset\":\"gnomad_r4\""), "{posted}");
    assert_eq!(result["source"], "gnomAD");
    assert_eq!(result["source_url"], GNOMAD_API);
    assert_eq!(result["found"], true);
    assert_eq!(result["variant"]["rsids"], json!(["rs7412"]));
    assert_eq!(
        result["url"],
        "https://gnomad.broadinstitute.org/variant/19-44908822-C-T?dataset=gnomad_r4"
    );
    assert!(!result.to_string().contains("synthetic-key"));
}

#[tokio::test]
async fn get_variant_treats_not_found_as_absence() {
    let app = Router::new().route(
        "/api",
        post(|| async { axum::Json(json!({"errors": [{"message": "Variant not found"}]})) }),
    );
    let (bio, server) = serve(app).await;
    let result = bio
        .call("get_variant", &json!({"variant_id": "1-1-A-T"}))
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["found"], false);
    assert!(result["variant"].is_null());
}

#[tokio::test]
async fn remaining_gnomad_tools_dispatch_through_native_bio_call() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = captured.clone();
    let app = Router::new().route(
        "/api",
        post(move |incoming: String| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(incoming.clone());
                let payload: Value = serde_json::from_str(&incoming).unwrap();
                let query = payload["query"].as_str().unwrap_or_default();
                let body = if query.contains("VariantSearch") {
                    json!({"data": {"variant_search": [{"variant_id": "19-44908822-C-T"}]}})
                } else if query.contains("GeneConstraint") {
                    json!({"data": {"gene": {
                        "gene_id": "ENSG00000141510", "symbol": "TP53",
                        "canonical_transcript_id": "ENST00000269305",
                        "chrom": "17", "start": 7661779, "stop": 7687550, "strand": "-",
                        "gnomad_constraint": {"pli": 0.99, "oe_lof_upper": 0.2, "obs_lof": 1, "exp_lof": 10.0}
                    }}})
                } else if query.contains("RegionVariants") {
                    json!({"data": {"region": {"variants": [
                        {"variant_id": "19-100-A-T", "pos": 100, "ref": "A", "alt": "T", "rsids": []},
                        {"variant_id": "19-90-C-G", "pos": 90, "ref": "C", "alt": "G", "rsids": ["rs9"]}
                    ]}}})
                } else if query.contains("Liftover") {
                    json!({"data": {"liftover": [{
                        "source": {"variant_id": "19-45412079-C-T", "reference_genome": "GRCh37"},
                        "liftover": {"variant_id": "19-44908822-C-T", "reference_genome": "GRCh38"},
                        "datasets": ["gnomad_r4"]
                    }]}})
                } else if query.contains("ClinvarVariants") {
                    json!({"data": {
                        "meta": {"clinvar_release_date": "2026-01-01"},
                        "gene": {"gene_id": "ENSG00000012048", "symbol": "BRCA1",
                            "clinvar_variants": [{"variant_id": "17-43094692-G-A", "clinvar_variation_id": "17661", "clinical_significance": "Pathogenic", "gold_stars": 3, "review_status": "reviewed by expert panel", "pos": 43094692, "in_gnomad": true}]}
                    }})
                } else if query.contains("StructuralVariantsGene") {
                    json!({"data": {"gene": {"gene_id": "ENSG00000141510", "symbol": "TP53",
                        "structural_variants": [{"variant_id": "DEL_chr17_1", "type": "DEL", "chrom": "17", "pos": 1, "end": 100, "ac": 2, "an": 100, "af": 0.02, "filters": ["PASS"]}]}}})
                } else if query.contains("query StructuralVariant") {
                    json!({"data": {"structural_variant": {
                        "variant_id": "DEL_chr17_1", "chrom": "17", "pos": 1, "end": 100, "type": "DEL",
                        "ac": 2, "an": 100, "af": 0.02, "qual": 20.0,
                        "algorithms": ["depth"], "evidence": ["RD"],
                        "consequences": [{"consequence": "LOF", "genes": ["TP53"]}]
                    }}})
                } else if query.contains("MitochondrialVariantsRegion") {
                    json!({"data": {"region": {"mitochondrial_variants": [{"variant_id": "M-3243-A-G", "pos": 3243, "ac_het": 1, "ac_hom": 0, "an": 50, "max_heteroplasmy": 0.2, "filters": []}]}}})
                } else {
                    json!({"data": {"gene": {
                        "gene_id": "ENSG00000130203", "symbol": "APOE", "chrom": "19", "start": 1, "stop": 2,
                        "variants": [{"variant_id": "19-44908822-C-T", "pos": 44908822, "ref": "C", "alt": "T", "rsids": ["rs7412"], "exome": {"ac": 1, "an": 10, "af": 0.1}}]
                    }}})
                };
                axum::Json(body)
            }
        }),
    );
    let (bio, server) = serve(app).await;
    let search = bio
        .call("search_variants", &json!({"query": "rs7412"}))
        .await
        .unwrap();
    let gene = bio
        .call("gene_variants", &json!({"gene_symbol": "APOE"}))
        .await
        .unwrap();
    let constraint = bio
        .call("gene_constraint", &json!({"gene_symbol": "TP53"}))
        .await
        .unwrap();
    let region = bio
        .call(
            "region_variants",
            &json!({"chrom": "19", "start": 1, "stop": 200}),
        )
        .await
        .unwrap();
    let lift = bio
        .call(
            "liftover_variant",
            &json!({"variant_id": "19-45412079-C-T", "source_build": "GRCh37"}),
        )
        .await
        .unwrap();
    let clinvar = bio
        .call("clinvar_variants", &json!({"gene_symbol": "BRCA1"}))
        .await
        .unwrap();
    let svs = bio
        .call("structural_variants", &json!({"gene_symbol": "TP53"}))
        .await
        .unwrap();
    let sv = bio
        .call("get_structural_variant", &json!({"sv_id": "DEL_chr17_1"}))
        .await
        .unwrap();
    let mito = bio
        .call(
            "mitochondrial_variants",
            &json!({"region_start": 3200, "region_stop": 3300}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(search["variant_ids"], json!(["19-44908822-C-T"]));
    assert_eq!(gene["n_variants"], 1);
    assert_eq!(constraint["constraint"]["pli"], 0.99);
    assert_eq!(region["variants"][0]["variant_id"], "19-90-C-G");
    assert_eq!(lift["n_results"], 1);
    assert_eq!(clinvar["clinvar_release_date"], "2026-01-01");
    assert_eq!(svs["n_variants"], 1);
    assert_eq!(sv["found"], true);
    assert_eq!(mito["region"], "M:3200-3300");
    assert_eq!(mito["n_variants"], 1);
}

#[tokio::test]
async fn clinvar_search_encodes_identity_and_reports_missing_and_truncation() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = captured.clone();
    let app = Router::new()
        .route(
            "/esearch.fcgi",
            post({
                let seen = seen.clone();
                move |incoming: String| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(format!("search {incoming}"));
                        axum::Json(
                            json!({"esearchresult": {"count": "3", "idlist": ["45122", "9"]}}),
                        )
                    }
                }
            }),
        )
        .route(
            "/esummary.fcgi",
            post({
                let seen = seen.clone();
                move |incoming: String| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(format!("summary {incoming}"));
                        axum::Json(json!({"result": {
                            "uids": ["45122"],
                            "45122": {
                                "uid": "45122",
                                "accession": "VCV000045122",
                                "title": "synthetic",
                                "germline_classification": {
                                    "description": "Pathogenic",
                                    "review_status": "practice guideline"
                                }
                            }
                        }}))
                    }
                }
            }),
        );
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "clinvar_search",
            &json!({"query": "TP53[gene] AND pathogenic[CLIN_SIG]", "max_records": 2}),
        )
        .await
        .unwrap();
    server.abort();
    let traffic = captured.lock().unwrap().join("\n");
    assert!(traffic.contains("term=TP53"), "{traffic}");
    assert!(
        traffic.contains("email=operator%40example.test"),
        "{traffic}"
    );
    assert!(
        traffic.contains("api_key=synthetic-key%26value"),
        "{traffic}"
    );
    assert!(traffic.contains("tool=wisp-science"), "{traffic}");
    assert_eq!(result["source"], "NCBI ClinVar");
    assert_eq!(result["total"], 3);
    assert_eq!(result["truncated"], true);
    assert_eq!(result["missing_uids"], json!(["9"]));
    assert_eq!(
        result["records"][0]["germline_classification"]["gold_stars"],
        4
    );
    assert!(!result.to_string().contains("synthetic-key"));
}

#[tokio::test]
async fn clinvar_records_resolve_vcv_and_unknown_rcv() {
    let app = Router::new()
        .route(
            "/esearch.fcgi",
            post(|incoming: String| async move {
                if incoming.contains("RCV999999999") {
                    axum::Json(json!({"esearchresult": {"count": "0", "idlist": []}}))
                } else {
                    axum::Json(json!({"esearchresult": {"count": "1", "idlist": ["9"]}}))
                }
            }),
        )
        .route(
            "/esummary.fcgi",
            post(|| async {
                axum::Json(json!({"result": {"uids": ["45122"], "45122": {
                    "uid": "45122", "accession": "VCV000045122", "title": "synthetic"
                }}}))
            }),
        );
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "clinvar_get_records",
            &json!({"accessions": ["VCV000045122", "RCV999999999"]}),
        )
        .await
        .unwrap();
    let rsid = bio
        .call("clinvar_variant_by_rsid", &json!({"rsid": "rs7412"}))
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["not_found"], json!(["RCV999999999"]));
    assert_eq!(
        result["records"][0]["requested_as"],
        json!(["VCV000045122"])
    );
    assert_eq!(rsid["rsid"], "rs7412");
}

#[tokio::test]
async fn dbsnp_lookup_and_region_search() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = captured.clone();
    let app = Router::new()
        .route(
            "/variation/v0/refsnp/{rsid}",
            get({
                let seen = seen.clone();
                move |Path(rsid): Path<String>| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(format!("refsnp {rsid}"));
                        if rsid == "999999999" {
                            StatusCode::NOT_FOUND.into_response()
                        } else {
                            axum::Json(json!({
                                "refsnp_id": rsid,
                                "primary_snapshot_data": {
                                    "variant_type": "snv",
                                    "placements_with_allele": [{
                                        "seq_id": "NC_000019.10",
                                        "is_ptlp": true,
                                        "placement_annot": {"seq_id_traits_by_assembly": [{
                                            "is_chromosome": true, "assembly_name": "GRCh38"
                                        }]},
                                        "alleles": [
                                            {"allele": {"spdi": {"seq_id": "NC_000019.10", "position": 10, "deleted_sequence": "A", "inserted_sequence": "A"}}},
                                            {"allele": {"spdi": {"seq_id": "NC_000019.10", "position": 10, "deleted_sequence": "A", "inserted_sequence": "G"}}}
                                        ]
                                    }],
                                    "allele_annotations": [{}, {}]
                                }
                            }))
                            .into_response()
                        }
                    }
                }
            }),
        )
        .route(
            "/esearch.fcgi",
            post({
                let seen = seen.clone();
                move |incoming: String| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(format!("search {incoming}"));
                        axum::Json(json!({"esearchresult": {"count": "5", "idlist": ["7412", "429358"]}}))
                    }
                }
            }),
        );
    let (bio, server) = serve(app).await;
    let records = bio
        .call(
            "dbsnp_get_rsids",
            &json!({"rsids": ["rs7412", "rs999999999"]}),
        )
        .await
        .unwrap();
    let region = bio
        .call(
            "dbsnp_search_by_region",
            &json!({"chrom": "chr19", "start": 44908000, "stop": 44909000, "max_rsids": 2}),
        )
        .await
        .unwrap();
    server.abort();
    let traffic = captured.lock().unwrap().join("\n");
    assert!(traffic.contains("refsnp 7412"), "{traffic}");
    assert!(traffic.contains("POSITION"), "{traffic}");
    assert!(
        traffic.contains("19%5BCHR%5D") || traffic.contains("19[CHR]"),
        "{traffic}"
    );
    assert_eq!(records["not_found"], json!(["rs999999999"]));
    assert_eq!(records["records"][0]["status"], "live");
    assert_eq!(region["truncated"], true);
    assert_eq!(region["rsids"], json!(["rs7412", "rs429358"]));
    assert_eq!(region["assembly"], "GRCh38");
}

#[tokio::test]
async fn rejects_rate_limits_missing_email_and_oversized_bodies() {
    let app_429 = Router::new().route(
        "/api",
        post(|| async {
            (
                StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", "60")],
                "secret-token",
            )
                .into_response()
        }),
    );
    let (bio, server) = serve(app_429).await;
    let error = bio
        .call("get_variant", &json!({"variant_id": "1-1-A-T"}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("HTTP 429"), "{error}");
    assert!(!error.contains("secret-token"));

    let app_big = Router::new().route(
        "/esearch.fcgi",
        post(|| async { (StatusCode::OK, " ".repeat(MAX_RESPONSE + 1)).into_response() }),
    );
    let (bio, server) = serve(app_big).await;
    let error = bio
        .call("clinvar_search", &json!({"query": "TP53"}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("exceeded 4 MiB"), "{error}");

    let no_email = NativeBio::test_client(
        &[],
        Http(reqwest::Client::builder().no_proxy().build().unwrap()),
    )
    .unwrap();
    let error = no_email
        .call("clinvar_search", &json!({"query": "TP53"}))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("contact email"), "{error}");

    let error = NativeBio::test_client(
        &[("NCBI_EMAIL".into(), "operator@example.test".into())],
        Http(reqwest::Client::builder().no_proxy().build().unwrap()),
    )
    .unwrap()
    .call(
        "mitochondrial_variants",
        &json!({"gene_symbol": "MT-TL1", "region_start": 1, "region_stop": 10}),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("not both"), "{error}");
}

#[test]
fn cadd_rejects_bare_version_mito_ref_eq_alt_span_and_unknown_fields() {
    assert!(cadd::require_version("v1.7").is_err());
    assert!(cadd::require_version("GRCh38-v1.7").is_ok());
    assert!(cadd::require_version("GRCh37-v1.6").is_ok());
    assert!(cadd::require_version("GRCh38-v1.7_inclAnno").is_ok());
    assert!(cadd::require_version("GRCh38-v1.7_inclanno").is_err());
    assert_eq!(cadd::require_chrom("chr4").unwrap(), "4");
    assert_eq!(cadd::require_chrom("X").unwrap(), "X");
    assert!(cadd::require_chrom("MT").is_err());
    assert!(cadd::require_chrom("M").is_err());
    assert!(cadd::require_chrom("chrM").is_err());
    assert_eq!(cadd::require_allele("a", "ref").unwrap(), "A");
    assert!(cadd::require_allele("N", "alt").is_err());
    assert!(cadd::require_span(1, 100).is_ok());
    assert!(cadd::require_span(1, 101).is_err());
    assert!(cadd::require_span(20, 10).is_err());
    assert!(serde_json::from_value::<cadd::PositionScores>(json!({
        "chrom": "4", "pos": 9001, "api_key": "secret"
    }))
    .is_err());
    assert!(serde_json::from_value::<cadd::VariantScore>(json!({
        "chrom": "4", "pos": 9001, "ref": "A", "alt": "T", "api_key": "secret"
    }))
    .is_err());
    assert!(serde_json::from_value::<cadd::RangeScores>(json!({
        "chrom": "4", "start": 1, "end": 2, "api_key": "secret"
    }))
    .is_err());
}

#[tokio::test]
async fn cadd_rejects_invalid_args_before_http() {
    let hits = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = hits.clone();
    let app = Router::new().route(
        "/{version}/{coord}",
        get(move |Path((version, coord)): Path<(String, String)>| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(format!("{version}/{coord}"));
                axum::Json(cadd_position_rows())
            }
        }),
    );
    let (bio, server) = cadd_serve(app).await;
    for (tool, args, needle) in [
        (
            "cadd_position_scores",
            json!({"chrom": "4", "pos": 9001, "version": "v1.7"}),
            "bare v1.7",
        ),
        (
            "cadd_position_scores",
            json!({"chrom": "MT", "pos": 9001}),
            "mitochondrial",
        ),
        (
            "cadd_variant_score",
            json!({"chrom": "4", "pos": 9001, "ref": "A", "alt": "A"}),
            "must differ",
        ),
        (
            "cadd_range_scores",
            json!({"chrom": "4", "start": 1, "end": 101}),
            "101 bp",
        ),
        (
            "cadd_position_scores",
            json!({"chrom": "4", "pos": 9001, "api_key": "secret"}),
            "invalid cadd_position_scores arguments",
        ),
    ] {
        let error = bio.call(tool, &args).await.unwrap_err().to_string();
        assert!(error.contains(needle), "{tool} {args}: {error}");
        assert!(!error.contains("secret"), "{error}");
    }
    server.abort();
    assert!(hits.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cadd_position_parses_capitalized_objects_sorts_and_reports_source_url() {
    let hits = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = hits.clone();
    let app = Router::new().route(
        "/{version}/{coord}",
        get(move |Path((version, coord)): Path<(String, String)>| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(format!("{version}/{coord}"));
                axum::Json(cadd_position_rows())
            }
        }),
    );
    let (bio, server) = cadd_serve(app).await;
    let result = bio
        .call(
            "cadd_position_scores",
            &json!({"chrom": "chr4", "pos": 9001}),
        )
        .await
        .unwrap();
    server.abort();
    let traffic = hits.lock().unwrap().clone();
    assert_eq!(traffic, vec!["GRCh38-v1.7/4:9001".to_string()]);
    assert_eq!(result["source"], "CADD");
    assert_eq!(result["source_url"], "https://cadd.gs.washington.edu/api");
    assert_eq!(
        result["query"],
        json!({"type": "position", "version": "GRCh38-v1.7", "chrom": "4", "pos": 9001})
    );
    assert_eq!(result["records"][0]["alt"], "C");
    assert_eq!(result["records"][1]["alt"], "G");
    assert_eq!(result["records"][2]["alt"], "T");
    assert_eq!(result["records"][0]["pos"], 9001);
    assert_eq!(result["records"][0]["raw_score"], "-0.251851");
    assert_eq!(result["records"][0]["phred"], "0.850");
}

#[tokio::test]
async fn cadd_variant_checks_reference_and_alt() {
    let app = Router::new().route(
        "/{version}/{coord}",
        get(
            |Path((_version, coord)): Path<(String, String)>| async move {
                assert_eq!(coord, "4:9001");
                axum::Json(cadd_two_alts())
            },
        ),
    );
    let (bio, server) = cadd_serve(app).await;
    let hit = bio
        .call(
            "cadd_variant_score",
            &json!({"chrom": "4", "pos": 9001, "ref": "A", "alt": "c"}),
        )
        .await
        .unwrap();
    assert_eq!(hit["query"]["type"], "variant");
    assert_eq!(hit["record"]["alt"], "C");
    assert_eq!(hit["record"]["raw_score"], "-0.251851");
    assert_eq!(hit["record"]["phred"], "0.850");
    assert_eq!(hit["source_url"], "https://cadd.gs.washington.edu/api");

    let wrong_ref = bio
        .call(
            "cadd_variant_score",
            &json!({"chrom": "4", "pos": 9001, "ref": "C", "alt": "T"}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(wrong_ref.contains("wrong build or typo"), "{wrong_ref}");
    assert!(wrong_ref.contains("query ref=C"), "{wrong_ref}");
    assert!(wrong_ref.contains("reference allele is A"), "{wrong_ref}");

    let missing_alt = bio
        .call(
            "cadd_variant_score",
            &json!({"chrom": "4", "pos": 9001, "ref": "A", "alt": "T"}),
        )
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(missing_alt.contains("alt=T"), "{missing_alt}");
    assert!(missing_alt.contains("alts present: C, G"), "{missing_alt}");
}

#[tokio::test]
async fn cadd_range_parses_header_rows_and_oversized_never_hits_http() {
    let hits = Arc::new(StdMutex::new(0u32));
    let seen = hits.clone();
    let app = Router::new().route(
        "/{version}/{coord}",
        get(move |Path((version, coord)): Path<(String, String)>| {
            let seen = seen.clone();
            async move {
                *seen.lock().unwrap() += 1;
                assert_eq!(version, "GRCh38-v1.7");
                assert_eq!(coord, "4:9001-9002");
                axum::Json(cadd_range_rows())
            }
        }),
    );
    let (bio, server) = cadd_serve(app).await;
    let result = bio
        .call(
            "cadd_range_scores",
            &json!({"chrom": "4", "start": 9001, "end": 9002}),
        )
        .await
        .unwrap();
    assert_eq!(result["n_records"], 3);
    assert_eq!(result["n_positions_scored"], 2);
    assert_eq!(result["span_bp"], 2);
    assert_eq!(result["truncated"], false);
    assert_eq!(result["records"][0]["pos"], 9001);
    assert_eq!(result["records"][0]["alt"], "C");
    assert_eq!(result["records"][0]["raw_score"], "-0.251851");
    assert_eq!(result["records"][1]["alt"], "G");
    assert_eq!(result["records"][2]["pos"], 9002);
    assert_eq!(*hits.lock().unwrap(), 1);

    let error = bio
        .call(
            "cadd_range_scores",
            &json!({"chrom": "4", "start": 1, "end": 101}),
        )
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("101 bp"), "{error}");
    assert_eq!(*hits.lock().unwrap(), 1);
}

#[tokio::test]
async fn cadd_empty_list_is_not_success() {
    let app = Router::new().route(
        "/{version}/{coord}",
        get(|| async { axum::Json(json!([])) }),
    );
    let (bio, server) = cadd_serve(app).await;
    let position = bio
        .call("cadd_position_scores", &json!({"chrom": "4", "pos": 9001}))
        .await
        .unwrap_err()
        .to_string();
    let variant = bio
        .call(
            "cadd_variant_score",
            &json!({"chrom": "4", "pos": 9001, "ref": "A", "alt": "T"}),
        )
        .await
        .unwrap_err()
        .to_string();
    let range = bio
        .call(
            "cadd_range_scores",
            &json!({"chrom": "4", "start": 9001, "end": 9002}),
        )
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(
        position.contains("no CADD rows for GRCh38-v1.7 4:9001"),
        "{position}"
    );
    assert!(
        variant.contains("no CADD rows for GRCh38-v1.7 4:9001"),
        "{variant}"
    );
    assert!(
        range.contains("no CADD rows for GRCh38-v1.7 4:9001-9002"),
        "{range}"
    );
}

#[tokio::test]
async fn cadd_http_errors_do_not_echo_bodies() {
    for status in [
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::TOO_MANY_REQUESTS,
    ] {
        let app = Router::new().route(
            "/{version}/{coord}",
            get(move || async move {
                (status, [("retry-after", "60")], "secret-token").into_response()
            }),
        );
        let (bio, server) = cadd_serve(app).await;
        let error = bio
            .call("cadd_position_scores", &json!({"chrom": "4", "pos": 9001}))
            .await
            .unwrap_err()
            .to_string();
        server.abort();
        assert!(
            error.contains(&format!("HTTP {}", status.as_u16())),
            "{error}"
        );
        assert!(!error.contains("secret-token"), "{error}");
    }
}

#[tokio::test]
async fn unknown_cadd_tool_fails_and_real_names_dispatch() {
    let app = Router::new().route(
        "/{version}/{coord}",
        get(
            |Path((_version, coord)): Path<(String, String)>| async move {
                if coord.contains('-') {
                    axum::Json(cadd_range_rows()).into_response()
                } else {
                    axum::Json(cadd_two_alts()).into_response()
                }
            },
        ),
    );
    let (bio, server) = cadd_serve(app).await;
    let unknown = bio
        .call(
            "cadd_get_variant_score",
            &json!({"chrom": "4", "pos": 9001}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        unknown.contains("unknown native biological tool"),
        "{unknown}"
    );

    let position = bio
        .call("cadd_position_scores", &json!({"chrom": "4", "pos": 9001}))
        .await
        .unwrap();
    let variant = bio
        .call(
            "cadd_variant_score",
            &json!({"chrom": "4", "pos": 9001, "ref": "A", "alt": "G"}),
        )
        .await
        .unwrap();
    let range = bio
        .call(
            "cadd_range_scores",
            &json!({"chrom": "4", "start": 9001, "end": 9002}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(position["records"].as_array().unwrap().len(), 2);
    assert_eq!(variant["record"]["alt"], "G");
    assert_eq!(range["n_records"], 3);
}
