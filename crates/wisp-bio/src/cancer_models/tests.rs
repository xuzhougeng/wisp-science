use super::*;
use crate::http::{Http, MAX_RESPONSE};
use axum::{
    extract::{Path, Query},
    http::{header::HeaderName, StatusCode, Uri},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

fn test_bio(base: &str) -> NativeBio {
    NativeBio::test_client(
        &[(
            "CBIOPORTAL_BASE_URL".into(),
            base.trim_end_matches('/').into(),
        )],
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

fn header_total(count: u64, body: impl IntoResponse) -> impl IntoResponse {
    (
        [(HeaderName::from_static("total-count"), count.to_string())],
        body,
    )
}

fn study(id: &str, cancer: &str) -> Value {
    json!({
        "studyId": id,
        "name": format!("Synthetic {id}"),
        "description": "Invented cohort used only in tests. ".repeat(20),
        "cancerTypeId": cancer,
        "cancerType": {"cancerTypeId": cancer, "name": "Synthetic carcinoma"},
        "referenceGenome": "hg19",
        "pmid": "00000001",
        "citation": "Synthetic et al.",
        "publicStudy": true,
        "groups": "PUBLIC",
        "importDate": "2020-01-01 00:00:00",
        "sequencedSampleCount": 100,
        "cnaSampleCount": 90,
        "structuralVariantCount": 4,
        "allSampleCount": 1
    })
}

fn gene() -> Value {
    json!({
        "entrezGeneId": 3845,
        "hugoGeneSymbol": "KRAS",
        "geneticEntityId": 1,
        "type": "protein-coding"
    })
}

fn mutation(sample: &str, change: &str, pos: i64) -> Value {
    json!({
        "sampleId": sample,
        "patientId": format!("{sample}-P"),
        "entrezGeneId": 3845,
        "molecularProfileId": "syn_brca_2020_mutations",
        "studyId": "syn_brca_2020",
        "proteinChange": change,
        "mutationType": "Missense_Mutation",
        "mutationStatus": "SOMATIC",
        "chr": "12",
        "startPosition": pos,
        "endPosition": pos,
        "referenceAllele": "C",
        "variantAllele": "T",
        "variantType": "SNP",
        "ncbiBuild": "GRCh37",
        "proteinPosStart": 12,
        "proteinPosEnd": 12,
        "tumorAltCount": 40,
        "tumorRefCount": 80,
        "refseqMrnaId": "NM_004985",
        "uniqueSampleKey": "should-not-leak"
    })
}

fn fixture_router() -> Router {
    Router::new()
        .route(
            "/studies",
            get(|uri: Uri, Query(query): Query<HashMap<String, String>>| async move {
                let _ = uri;
                let keyword = query.get("keyword").cloned().unwrap_or_default();
                let page_size = query
                    .get("pageSize")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(50);
                let mut rows = vec![study("syn_brca_2020", "brca"), study("syn_luad_2021", "luad")];
                if keyword == "secret-token" {
                    return (StatusCode::OK, "should-not-echo").into_response();
                }
                if !keyword.is_empty() {
                    rows.retain(|row| {
                        row["name"]
                            .as_str()
                            .unwrap_or("")
                            .to_ascii_lowercase()
                            .contains(&keyword.to_ascii_lowercase())
                    });
                }
                let total = rows.len() as u64;
                rows.truncate(page_size);
                header_total(total, axum::Json(rows)).into_response()
            }),
        )
        .route(
            "/studies/{study_id}",
            get(|Path(study_id): Path<String>| async move {
                if study_id == "missing_study" {
                    return StatusCode::NOT_FOUND.into_response();
                }
                if study_id != "syn_brca_2020" && study_id != "syn_luad_2021" {
                    return StatusCode::NOT_FOUND.into_response();
                }
                axum::Json(study(&study_id, if study_id.contains("luad") {
                    "luad"
                } else {
                    "brca"
                }))
                .into_response()
            }),
        )
        .route(
            "/studies/{study_id}/samples",
            get(|Path(study_id): Path<String>, Query(query): Query<HashMap<String, String>>| async move {
                if study_id == "missing_study" {
                    return StatusCode::NOT_FOUND.into_response();
                }
                if query.get("projection").map(String::as_str) == Some("META") {
                    return header_total(42, ()).into_response();
                }
                header_total(42, axum::Json(json!([]))).into_response()
            }),
        )
        .route(
            "/studies/{study_id}/patients",
            get(|Query(query): Query<HashMap<String, String>>| async move {
                if query.get("projection").map(String::as_str) == Some("META") {
                    return header_total(40, ()).into_response();
                }
                header_total(40, axum::Json(json!([]))).into_response()
            }),
        )
        .route(
            "/studies/{study_id}/molecular-profiles",
            get(|Path(study_id): Path<String>| async move {
                if study_id == "syn_luad_2021" {
                    return axum::Json(json!([{
                        "molecularProfileId": "syn_luad_2021_rna",
                        "molecularAlterationType": "MRNA_EXPRESSION",
                        "datatype": "CONTINUOUS",
                        "name": "RNA",
                        "description": "Invented RNA profile"
                    }]))
                    .into_response();
                }
                axum::Json(json!([
                    {
                        "molecularProfileId": format!("{study_id}_gistic"),
                        "molecularAlterationType": "COPY_NUMBER_ALTERATION",
                        "datatype": "DISCRETE",
                        "name": "GISTIC",
                        "description": "Invented discrete CNA"
                    },
                    {
                        "molecularProfileId": format!("{study_id}_mutations"),
                        "molecularAlterationType": "MUTATION_EXTENDED",
                        "datatype": "MAF",
                        "name": "Mutations",
                        "description": "Invented mutations"
                    }
                ]))
                .into_response()
            }),
        )
        .route(
            "/studies/{study_id}/clinical-attributes",
            get(|Path(study_id): Path<String>, Query(query): Query<HashMap<String, String>>| async move {
                if study_id == "missing_study" {
                    return StatusCode::NOT_FOUND.into_response();
                }
                let page_size = query
                    .get("pageSize")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(100);
                let rows = vec![
                    json!({
                        "clinicalAttributeId": "OS_MONTHS",
                        "displayName": "Overall Survival (Months)",
                        "description": "Invented survival months",
                        "datatype": "NUMBER",
                        "patientAttribute": true,
                        "priority": "1",
                        "studyId": study_id
                    }),
                    json!({
                        "clinicalAttributeId": "OS_STATUS",
                        "displayName": "Overall Survival Status",
                        "description": "Invented survival status",
                        "datatype": "STRING",
                        "patientAttribute": true,
                        "priority": "1",
                        "studyId": study_id
                    }),
                    json!({
                        "clinicalAttributeId": "SAMPLE_TYPE",
                        "displayName": "Sample Type",
                        "description": "Invented sample type",
                        "datatype": "STRING",
                        "patientAttribute": false,
                        "priority": "1",
                        "studyId": study_id
                    }),
                ];
                let total = rows.len() as u64;
                let page: Vec<_> = rows.into_iter().take(page_size).collect();
                header_total(total, axum::Json(page)).into_response()
            }),
        )
        .route(
            "/genes/{gene_id}",
            get(|Path(gene_id): Path<String>| async move {
                if gene_id == "NOTAGENE" {
                    return StatusCode::NOT_FOUND.into_response();
                }
                axum::Json(gene()).into_response()
            }),
        )
        .route(
            "/sample-lists/{list_id}",
            get(|Path(list_id): Path<String>| async move {
                if list_id.ends_with("_all") && !list_id.starts_with("syn_luad_2021") {
                    return axum::Json(json!({
                        "sampleListId": list_id,
                        "studyId": list_id.trim_end_matches("_all"),
                        "category": "all_cases_in_study",
                        "sampleCount": 100
                    }))
                    .into_response();
                }
                StatusCode::NOT_FOUND.into_response()
            }),
        )
        .route(
            "/molecular-profiles/{profile_id}/mutations",
            get(
                |Path(profile_id): Path<String>, Query(query): Query<HashMap<String, String>>| async move {
                    if !profile_id.ends_with("_mutations") {
                        return StatusCode::NOT_FOUND.into_response();
                    }
                    let page_size = query
                        .get("pageSize")
                        .and_then(|value| value.parse::<usize>().ok())
                        .unwrap_or(50);
                    let rows = vec![
                        mutation("SYN-1", "G12D", 25398284),
                        mutation("SYN-2", "G12V", 25398285),
                        mutation("SYN-3", "G12D", 25398284),
                    ];
                    let total = rows.len() as u64;
                    let page: Vec<_> = rows.into_iter().take(page_size).collect();
                    header_total(total, axum::Json(page)).into_response()
                },
            ),
        )
        .route(
            "/molecular-profiles/{profile_id}/discrete-copy-number/fetch",
            post(
                |Path(profile_id): Path<String>,
                 Query(query): Query<HashMap<String, String>>,
                 body: String| async move {
                    if !profile_id.ends_with("_gistic") {
                        return StatusCode::NOT_FOUND.into_response();
                    }
                    let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                    if parsed.get("entrezGeneIds").is_none() {
                        return StatusCode::BAD_REQUEST.into_response();
                    }
                    let event = query
                        .get("discreteCopyNumberEventType")
                        .map(String::as_str)
                        .unwrap_or("HOMDEL_AND_AMP");
                    let mut rows = vec![
                        json!({
                            "sampleId": "SYN-1",
                            "patientId": "SYN-P1",
                            "entrezGeneId": 3845,
                            "molecularProfileId": profile_id,
                            "studyId": "syn_brca_2020",
                            "alteration": 2
                        }),
                        json!({
                            "sampleId": "SYN-4",
                            "patientId": "SYN-P4",
                            "entrezGeneId": 3845,
                            "molecularProfileId": profile_id,
                            "studyId": "syn_brca_2020",
                            "alteration": -2
                        }),
                    ];
                    if event == "AMP" {
                        rows.retain(|row| row["alteration"] == 2);
                    }
                    header_total(rows.len() as u64, axum::Json(rows)).into_response()
                },
            ),
        )
}

#[test]
fn catalog_registers_eleven_cancer_models_tools() {
    let tools: Vec<_> = catalog()
        .into_iter()
        .map(|(domain, schema)| (domain, schema.function.name, schema.function.description))
        .collect();
    let names: Vec<_> = tools
        .iter()
        .map(|(domain, name, _)| (*domain, name.clone()))
        .collect();
    assert_eq!(
        names,
        vec![
            ("cancer-models", "cbioportal_clinical_attributes".into()),
            ("cancer-models", "cbioportal_cna_in_gene".into()),
            ("cancer-models", "cbioportal_get_study".into()),
            ("cancer-models", "cbioportal_list_studies".into()),
            ("cancer-models", "cbioportal_mutation_frequency".into()),
            ("cancer-models", "cbioportal_mutations_in_gene".into()),
            ("cancer-models", "gene_dependencies".into()),
            ("cancer-models", "get_model".into()),
            ("cancer-models", "list_models".into()),
            ("cancer-models", "search_genes".into()),
            ("cancer-models", "search_models".into()),
        ]
    );
    for name in [
        "cbioportal_clinical_attributes",
        "cbioportal_cna_in_gene",
        "cbioportal_get_study",
        "cbioportal_list_studies",
        "cbioportal_mutation_frequency",
        "cbioportal_mutations_in_gene",
        "gene_dependencies",
        "get_model",
        "list_models",
        "search_genes",
        "search_models",
    ] {
        assert!(crate::contains_tool(name), "{name}");
        assert_eq!(crate::domain_for_tool(name), Some("cancer-models"));
    }
    assert!(crate::package_selects("mcp_cancer_models", "cancer-models"));
    assert!(crate::selected_by_package("mcp_cancer_models"));
    for (domain, name, description) in &tools {
        if matches!(
            name.as_str(),
            "gene_dependencies" | "get_model" | "list_models" | "search_genes" | "search_models"
        ) {
            assert_eq!(*domain, "cancer-models");
            assert!(
                description.contains("non-commercial"),
                "{name} missing usage notice"
            );
            assert!(description.contains("depmap@sanger.ac.uk"), "{name}");
        }
    }
}

#[test]
fn rejects_unbounded_or_malformed_arguments() {
    for args in [
        json!({"study_id": ""}),
        json!({"study_id": "bad/id"}),
        json!({"study_id": "id?x=1"}),
        json!({"study_id": "syn_brca_2020", "max_records": 0}),
        json!({"study_id": "syn_brca_2020", "max_records": 201}),
        json!({"study_id": "syn_brca_2020", "api_key": "secret"}),
    ] {
        let parsed = serde_json::from_value::<ClinicalAttributes>(args.clone());
        match parsed {
            Ok(parsed) => assert!(
                require_id(&parsed.study_id, "study_id", 128).is_err()
                    || bound_page(parsed.max_records).is_err(),
                "{args}"
            ),
            Err(_) => {}
        }
    }
    assert!(require_ids(&[], 12, "study_id").is_err());
    let too_many: Vec<String> = (0..13).map(|i| format!("syn_{i}")).collect();
    assert!(require_ids(&too_many, 12, "study_id").is_err());
    assert!(normalize_event_type("nope").is_err());
    assert_eq!(normalize_event_type("amp").unwrap(), "AMP");
    assert!(cna_label(2) == Some("amplification"));
    let long = "x".repeat(300);
    let trimmed = trim_text(&long, 256);
    assert!(trimmed.ends_with("..."));
    assert!(trimmed.len() <= 256);
}

#[tokio::test]
async fn list_studies_pages_source_urls_and_client_side_cancer_type() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = captured.clone();
    let app = Router::new().route(
        "/studies",
        get(move |uri: Uri| {
            let seen = seen.clone();
            async move {
                seen.lock()
                    .unwrap()
                    .push(uri.query().unwrap_or("").to_string());
                header_total(
                    3,
                    axum::Json(json!([
                        study("syn_brca_2020", "brca"),
                        study("syn_luad_2021", "luad")
                    ])),
                )
            }
        }),
    );
    let (bio, server) = serve(app).await;
    let listed = bio
        .call(
            "cbioportal_list_studies",
            &json!({"keyword": "syn & brca", "max_records": 1}),
        )
        .await
        .unwrap();
    let filtered = bio
        .call(
            "cbioportal_list_studies",
            &json!({"cancer_type_id": "LUAD", "max_records": 50}),
        )
        .await
        .unwrap();
    server.abort();
    let queries = captured.lock().unwrap().clone();
    assert!(
        queries.iter().any(|query| query.contains("keyword=syn")),
        "{queries:?}"
    );
    assert!(
        queries
            .iter()
            .any(|query| query.contains("projection=DETAILED")),
        "{queries:?}"
    );
    assert!(
        queries.iter().any(|query| query.contains("pageSize=1")),
        "{queries:?}"
    );
    assert_eq!(listed["source"], "cBioPortal");
    assert_eq!(listed["source_url"], API);
    assert_eq!(listed["total"], 3);
    assert_eq!(listed["returned"], 1);
    assert_eq!(listed["truncated"], true);
    assert_eq!(
        listed["studies"][0]["url"],
        "https://www.cbioportal.org/study/summary?id=syn_brca_2020"
    );
    assert!(listed["studies"][0]["description"]
        .as_str()
        .unwrap()
        .ends_with("..."));
    assert!(listed["studies"][0].get("all_sample_count").is_none());
    assert_eq!(filtered["returned"], 1);
    assert_eq!(filtered["studies"][0]["study_id"], "syn_luad_2021");
}

#[tokio::test]
async fn get_study_uses_meta_counts_and_fails_unknown_ids() {
    let (bio, server) = serve(fixture_router()).await;
    let found = bio
        .call(
            "cbioportal_get_study",
            &json!({"study_id": "syn_brca_2020"}),
        )
        .await
        .unwrap();
    let missing = bio
        .call(
            "cbioportal_get_study",
            &json!({"study_id": "missing_study"}),
        )
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert_eq!(found["study_id"], "syn_brca_2020");
    assert_eq!(found["sample_count"], 42);
    assert_eq!(found["patient_count"], 40);
    assert_eq!(found["sequenced_sample_count"], 100);
    assert_eq!(found["source_url"], API);
    assert_eq!(
        found["url"],
        "https://www.cbioportal.org/study/summary?id=syn_brca_2020"
    );
    let profiles = found["molecular_profiles"].as_array().unwrap();
    assert_eq!(profiles.len(), 2);
    assert_eq!(profiles[0]["molecular_profile_id"], "syn_brca_2020_gistic");
    assert!(missing.contains("was not found"), "{missing}");
}

#[tokio::test]
async fn mutations_frequency_and_cna_dispatch_through_native_bio_call() {
    let (bio, server) = serve(fixture_router()).await;
    let mutations = bio
        .call(
            "cbioportal_mutations_in_gene",
            &json!({
                "gene_symbol": "kras",
                "study_id": "syn_brca_2020",
                "max_records": 2
            }),
        )
        .await
        .unwrap();
    let frequency = bio
        .call(
            "cbioportal_mutation_frequency",
            &json!({
                "gene_symbol": "KRAS",
                "study_ids": ["syn_brca_2020", "missing_study", "syn_luad_2021"]
            }),
        )
        .await
        .unwrap();
    let cna = bio
        .call(
            "cbioportal_cna_in_gene",
            &json!({
                "gene_symbol": "KRAS",
                "study_id": "syn_brca_2020",
                "event_type": "HOMDEL_AND_AMP",
                "max_records": 1
            }),
        )
        .await
        .unwrap();
    let clinical = bio
        .call(
            "cbioportal_clinical_attributes",
            &json!({"study_id": "syn_brca_2020", "max_records": 2}),
        )
        .await
        .unwrap();
    let unknown_gene = bio
        .call(
            "cbioportal_mutations_in_gene",
            &json!({"gene_symbol": "NOTAGENE", "study_id": "syn_brca_2020"}),
        )
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert_eq!(mutations["total"], 3);
    assert_eq!(mutations["returned"], 2);
    assert_eq!(mutations["truncated"], true);
    assert_eq!(mutations["mutated_sample_count"], 2);
    assert_eq!(mutations["gene"]["symbol"], "KRAS");
    assert_eq!(mutations["gene"]["entrez_gene_id"], 3845);
    assert!(mutations.to_string().contains("G12D"));
    assert!(!mutations.to_string().contains("uniqueSampleKey"));
    assert_eq!(
        mutations["study_url"],
        "https://www.cbioportal.org/study/summary?id=syn_brca_2020"
    );
    assert_eq!(frequency["unknown_studies"], json!(["missing_study"]));
    assert_eq!(frequency["no_mutation_data"], json!(["syn_luad_2021"]));
    assert_eq!(frequency["count"], 1);
    assert_eq!(frequency["frequencies"][0]["mutated_samples"], 3);
    assert_eq!(frequency["frequencies"][0]["sequenced_samples"], 100);
    assert_eq!(frequency["frequencies"][0]["frequency"], 0.03);
    assert_eq!(cna["returned"], 1);
    assert_eq!(cna["truncated"], true);
    assert_eq!(cna["alteration_counts"]["amplification"], 1);
    assert_eq!(cna["alteration_counts"]["deep_deletion"], 1);
    assert_eq!(cna["events"][0]["alteration_label"], "amplification");
    assert_eq!(clinical["has_overall_survival"], true);
    assert_eq!(clinical["truncated"], true);
    assert_eq!(clinical["total"], 3);
    assert_eq!(clinical["returned"], 2);
    assert!(clinical["survival_attributes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|id| id == "OS_STATUS"));
    assert!(unknown_gene.contains("was not found"), "{unknown_gene}");
}

#[tokio::test]
async fn cna_posts_json_gene_filter() {
    let captured = Arc::new(StdMutex::new(String::new()));
    let body = captured.clone();
    let app = Router::new()
        .route("/genes/{gene_id}", get(|| async { axum::Json(gene()) }))
        .route(
            "/studies/{study_id}/molecular-profiles",
            get(|| async {
                axum::Json(json!([{
                    "molecularProfileId": "syn_brca_2020_gistic",
                    "molecularAlterationType": "COPY_NUMBER_ALTERATION",
                    "datatype": "DISCRETE"
                }]))
            }),
        )
        .route(
            "/sample-lists/{list_id}",
            get(|| async { axum::Json(json!({"sampleListId": "syn_brca_2020_all"})) }),
        )
        .route(
            "/molecular-profiles/{profile_id}/discrete-copy-number/fetch",
            post(move |uri: Uri, incoming: String| {
                let body = body.clone();
                async move {
                    *body.lock().unwrap() = format!("{} {incoming}", uri.query().unwrap_or(""));
                    axum::Json(json!([]))
                }
            }),
        );
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "cbioportal_cna_in_gene",
            &json!({
                "gene_symbol": "KRAS",
                "study_id": "syn_brca_2020",
                "event_type": "AMP"
            }),
        )
        .await
        .unwrap();
    server.abort();
    let traffic = captured.lock().unwrap().clone();
    assert!(
        traffic.contains("discreteCopyNumberEventType=AMP"),
        "{traffic}"
    );
    assert!(traffic.contains("\"entrezGeneIds\":[3845]"), "{traffic}");
    assert!(
        traffic.contains("\"sampleListId\":\"syn_brca_2020_all\""),
        "{traffic}"
    );
    assert_eq!(result["event_type"], "AMP");
    assert_eq!(result["returned"], 0);
    assert_eq!(result["source_url"], API);
}

#[tokio::test]
async fn rejects_rate_limits_malformed_json_and_oversized_bodies_without_echoing_secrets() {
    for (status, body, expected) in [
        (
            StatusCode::TOO_MANY_REQUESTS,
            "secret-token".into(),
            "HTTP 429",
        ),
        (StatusCode::OK, "{not-json".into(), "invalid JSON"),
        (
            StatusCode::OK,
            " ".repeat(MAX_RESPONSE + 1),
            "exceeded 4 MiB",
        ),
    ] {
        let app = Router::new().route(
            "/studies",
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
            .call("cbioportal_list_studies", &json!({"keyword": "glioma"}))
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
async fn missing_meta_count_is_null_not_zero() {
    let app = Router::new()
        .route(
            "/studies/{study_id}",
            get(|| async { axum::Json(study("syn_brca_2020", "brca")) }),
        )
        .route(
            "/studies/{study_id}/samples",
            get(|| async { axum::Json(json!([])) }),
        )
        .route(
            "/studies/{study_id}/patients",
            get(|| async { axum::Json(json!([])) }),
        )
        .route(
            "/studies/{study_id}/molecular-profiles",
            get(|| async { axum::Json(json!([])) }),
        );
    let (bio, server) = serve(app).await;
    let result = bio
        .call(
            "cbioportal_get_study",
            &json!({"study_id": "syn_brca_2020"}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["sample_count"], Value::Null);
    assert_eq!(result["patient_count"], Value::Null);
    assert_eq!(result["sequenced_sample_count"], 100);
}

fn cmp_test_bio(base: &str) -> NativeBio {
    NativeBio::test_client(
        &[("CMP_BASE_URL".into(), base.trim_end_matches('/').into())],
        Http(reqwest::Client::builder().no_proxy().build().unwrap()),
    )
    .unwrap()
}

async fn cmp_serve(app: Router) -> (NativeBio, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (cmp_test_bio(&endpoint), task)
}

fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(value) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn decode_query(query: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        out.insert(percent_decode(key), percent_decode(value));
    }
    out
}

fn cmp_model(id: &str, names: &[&str]) -> Value {
    json!({
        "id": id,
        "type": "model",
        "attributes": {
            "names": names,
            "model_type": "Cell Line",
            "growth_properties": "Adherent",
            "model_treatment": "Naive",
            "ploidy_wes": 3.1,
            "ploidy_wgs": 3.2,
            "mutations_per_mb": 8.5,
            "crispr_ko_available": true,
            "mutations_available": true,
            "rnaseq_available": false
        }
    })
}

fn jsonapi_list(rows: Vec<Value>, total: usize) -> Value {
    json!({"data": rows, "meta": {"count": total}})
}

fn paginate(rows: &[Value], query: &str) -> Value {
    let params = decode_query(query);
    let size = params
        .get("page[size]")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(30);
    let number = params
        .get("page[number]")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1);
    let start = size.saturating_mul(number.saturating_sub(1));
    jsonapi_list(
        rows.iter().skip(start).take(size).cloned().collect(),
        rows.len(),
    )
}

fn cmp_gene(id: &str, symbol: &str) -> Value {
    json!({
        "id": id,
        "type": "gene",
        "attributes": {
            "symbol": symbol,
            "hgnc_id": "HGNC:1",
            "hgnc_status": "Approved",
            "location": "12p12.1",
            "cancer_driver": true,
            "tumour_suppressor": false,
            "in_yusa_lib": true
        }
    })
}

fn crispr_row(id: &str, model_id: &str, source: &str, bf: f64) -> Value {
    json!({
        "id": id,
        "type": "crispr_ko",
        "attributes": {
            "source": source,
            "bf": bf,
            "bf_scaled": 0.5,
            "fc_clean": -0.4,
            "fc_clean_qn": "-0.3",
            "mageck_fdr": 0.01,
            "qc_pass": true
        },
        "relationships": {
            "gene": {"data": {"type": "gene", "id": "SIDG00001"}},
            "model": {"data": {"type": "model", "id": model_id}}
        }
    })
}

#[tokio::test]
async fn rejects_empty_query_bounds_extra_keys_and_pathy_ids() {
    let bio = cmp_test_bio("http://127.0.0.1:1");
    let empty_search = bio
        .call("search_models", &json!({"query": ""}))
        .await
        .unwrap_err()
        .to_string();
    let empty_genes = bio
        .call("search_genes", &json!({"query": "   "}))
        .await
        .unwrap_err()
        .to_string();
    let zero = bio
        .call("list_models", &json!({"max_records": 0}))
        .await
        .unwrap_err()
        .to_string();
    let too_many = bio
        .call("list_models", &json!({"max_records": 201}))
        .await
        .unwrap_err()
        .to_string();
    let extra_key = bio
        .call(
            "list_models",
            &json!({"tissue": "Lung", "api_key": "secret"}),
        )
        .await
        .unwrap_err()
        .to_string();
    let pathy = bio
        .call("get_model", &json!({"model_id_or_name": "SIDM00001/../x"}))
        .await
        .unwrap_err()
        .to_string();
    let pathy_search = bio
        .call("search_models", &json!({"query": "lung?x=1"}))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        empty_search.contains("required") || empty_search.contains("invalid"),
        "{empty_search}"
    );
    assert!(
        empty_genes.contains("required")
            || empty_genes.contains("unsupported")
            || empty_genes.contains("invalid"),
        "{empty_genes}"
    );
    assert!(zero.contains("max_records"), "{zero}");
    assert!(too_many.contains("max_records"), "{too_many}");
    assert!(extra_key.contains("invalid"), "{extra_key}");
    assert!(!extra_key.contains("secret"), "{extra_key}");
    assert!(
        pathy.contains("path") || pathy.contains("unsupported"),
        "{pathy}"
    );
    assert!(pathy_search.contains("path"), "{pathy_search}");
    assert!(super::sanger::require_label("Small Cell Lung Carcinoma", "cancer_type").is_ok());
    assert!(super::sanger::require_label("Lung/Other", "tissue").is_err());
    assert!(super::sanger::require_label("x".repeat(300).as_str(), "query").is_err());
}

#[tokio::test]
async fn list_models_pages_filters_and_reports_total_vs_truncated() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = captured.clone();
    let app = Router::new().route(
        "/models",
        get(move |uri: Uri| {
            let seen = seen.clone();
            async move {
                let query = uri.query().unwrap_or("").to_string();
                seen.lock().unwrap().push(query.clone());
                let params = decode_query(&query);
                let mut rows: Vec<Value> = (1..=150)
                    .map(|i| cmp_model(&format!("SIDM{i:05}"), &["SYN-LUNG"]))
                    .collect();
                if let Some(filter) = params.get("filter") {
                    if filter.contains("Lung") {
                        rows.truncate(40);
                    }
                    if filter.contains("Small Cell Lung Carcinoma") {
                        rows.truncate(12);
                    }
                }
                axum::Json(paginate(&rows, &query))
            }
        }),
    );
    let (bio, server) = cmp_serve(app).await;
    let capped = bio
        .call("list_models", &json!({"max_records": 50}))
        .await
        .unwrap();
    let filtered = bio
        .call(
            "list_models",
            &json!({
                "tissue": "Lung",
                "cancer_type": "Small Cell Lung Carcinoma",
                "max_records": 50
            }),
        )
        .await
        .unwrap();
    let walked = bio
        .call("list_models", &json!({"max_records": 200}))
        .await
        .unwrap();
    server.abort();
    let queries = captured.lock().unwrap().clone();
    let decoded: Vec<HashMap<String, String>> = queries.iter().map(|q| decode_query(q)).collect();
    assert!(
        decoded
            .iter()
            .any(|q| q.get("page[size]").map(String::as_str) == Some("100")),
        "{queries:?}"
    );
    assert!(
        decoded
            .iter()
            .any(|q| q.get("page[number]").map(String::as_str) == Some("1")),
        "{queries:?}"
    );
    assert!(
        decoded
            .iter()
            .any(|q| q.get("page[number]").map(String::as_str) == Some("2")),
        "{queries:?}"
    );
    let filter = decoded
        .iter()
        .find_map(|q| q.get("filter").cloned())
        .expect("filter");
    let parsed: Value = serde_json::from_str(&filter).unwrap();
    assert!(filter.contains("\"op\":\"has\""), "{filter}");
    assert!(filter.contains("Lung"), "{filter}");
    assert!(filter.contains("Small Cell Lung Carcinoma"), "{filter}");
    assert_eq!(parsed.as_array().map(Vec::len), Some(2));
    assert_eq!(capped["source"], "Cell Model Passports");
    assert_eq!(
        capped["source_url"],
        "https://api.cellmodelpassports.sanger.ac.uk"
    );
    assert_eq!(capped["total"], 150);
    assert_eq!(capped["returned"], 50);
    assert_eq!(capped["truncated"], true);
    assert_eq!(capped["models"][0]["model_id"], "SIDM00001");
    assert_eq!(capped["models"][0]["names"], json!(["SYN-LUNG"]));
    assert_eq!(capped["models"][0]["crispr_ko_available"], true);
    assert_eq!(filtered["tissue"], "Lung");
    assert_eq!(filtered["cancer_type"], "Small Cell Lung Carcinoma");
    assert_eq!(filtered["total"], 12);
    assert_eq!(filtered["returned"], 12);
    assert_eq!(filtered["truncated"], false);
    assert_eq!(walked["returned"], 150);
    assert_eq!(walked["truncated"], false);
    assert!(queries.len() <= 8, "{queries:?}");
}

#[tokio::test]
async fn get_model_uses_include_and_fails_unknown_or_ambiguous_names() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = captured.clone();
    let app = Router::new()
        .route(
            "/models/{model_id}",
            get({
                let seen = seen.clone();
                move |Path(model_id): Path<String>, uri: Uri| {
                    let seen = seen.clone();
                    async move {
                        seen.lock()
                            .unwrap()
                            .push(format!("{}?{}", uri.path(), uri.query().unwrap_or("")));
                        if model_id == "SIDM99999" {
                            return axum::Json(json!({"data": null})).into_response();
                        }
                        if model_id != "SIDM00001" {
                            return StatusCode::NOT_FOUND.into_response();
                        }
                        axum::Json(json!({
                            "data": {
                                "id": "SIDM00001",
                                "type": "model",
                                "attributes": {
                                    "names": ["SYN-B", "syn-a"],
                                    "model_type": "Cell Line",
                                    "growth_properties": "Adherent",
                                    "model_treatment": "Naive",
                                    "ploidy_wes": 3.1,
                                    "ploidy_wgs": 3.2,
                                    "mutations_per_mb": 8.5,
                                    "crispr_ko_available": true,
                                    "mutations_available": true,
                                    "rnaseq_available": false
                                },
                                "relationships": {
                                    "sample": {"data": {"type": "sample", "id": "SIDS00001"}}
                                },
                                "links": {"self": "https://example.invalid/secret"}
                            },
                            "included": [
                                {"id": "SIDS00001", "type": "sample", "attributes": {}},
                                {"id": "t1", "type": "tissue", "attributes": {"name": "Lung"}},
                                {"id": "c1", "type": "cancer_type", "attributes": {"name": "Small Cell Lung Carcinoma"}},
                                {"id": "msi-old", "type": "model_msi_status", "attributes": {"msi_status": "MSS", "current": false}},
                                {"id": "msi-now", "type": "model_msi_status", "attributes": {"msi_status": "MSI", "current": true}}
                            ],
                            "jsonapi": {"version": "1.0"},
                            "links": {"self": "https://example.invalid/secret"}
                        }))
                        .into_response()
                    }
                }
            }),
        )
        .route(
            "/models",
            get({
                let seen = seen.clone();
                move |uri: Uri| {
                    let seen = seen.clone();
                    async move {
                        let query = uri.query().unwrap_or("").to_string();
                        seen.lock().unwrap().push(query.clone());
                        let params = decode_query(&query);
                        let filter = params.get("filter").cloned().unwrap_or_default();
                        if filter.contains("SYN-DUP") {
                            return axum::Json(json!({
                                "data": [
                                    cmp_model("SIDM00001", &["SYN-DUP"]),
                                    cmp_model("SIDM00002", &["SYN-DUP"])
                                ]
                            }))
                            .into_response();
                        }
                        if filter.contains("MISSING") {
                            return axum::Json(json!({"data": []})).into_response();
                        }
                        StatusCode::NOT_FOUND.into_response()
                    }
                }
            }),
        );
    let (bio, server) = cmp_serve(app).await;
    let found = bio
        .call("get_model", &json!({"model_id_or_name": "sidm00001"}))
        .await
        .unwrap();
    let unknown = bio
        .call("get_model", &json!({"model_id_or_name": "SIDM99999"}))
        .await
        .unwrap_err()
        .to_string();
    let missing_name = bio
        .call("get_model", &json!({"model_id_or_name": "MISSING"}))
        .await
        .unwrap_err()
        .to_string();
    let ambiguous = bio
        .call("get_model", &json!({"model_id_or_name": "SYN-DUP"}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    let traffic = captured.lock().unwrap().clone();
    assert!(
        traffic
            .iter()
            .any(|row| row.contains("include=sample.tissue")),
        "{traffic:?}"
    );
    assert_eq!(found["model_id"], "SIDM00001");
    assert_eq!(found["names"], json!(["syn-a", "SYN-B"]));
    assert_eq!(found["tissue"], "Lung");
    assert_eq!(found["cancer_type"], "Small Cell Lung Carcinoma");
    assert_eq!(found["msi_status"], "MSI");
    assert_eq!(found["sample_id"], "SIDS00001");
    assert_eq!(found["source"], "Cell Model Passports");
    assert!(found.get("links").is_none());
    assert!(found.get("jsonapi").is_none());
    assert!(!found.to_string().contains("example.invalid"));
    assert!(unknown.contains("was not found"), "{unknown}");
    assert!(missing_name.contains("was not found"), "{missing_name}");
    assert!(ambiguous.contains("ambiguous"), "{ambiguous}");
    assert!(ambiguous.contains("SIDM00001"), "{ambiguous}");
    assert!(ambiguous.contains("SIDM00002"), "{ambiguous}");
}

#[tokio::test]
async fn search_models_keeps_type_model_only() {
    let app = Router::new().route(
        "/search/{query}",
        get(|Path(query): Path<String>| async move {
            assert_eq!(query, "SYN");
            axum::Json(json!({
                "data": [
                    cmp_model("SIDM00002", &["SYN-2"]),
                    {"id": "SIDG00001", "type": "gene", "attributes": {"symbol": "SYN1"}},
                    cmp_model("SIDM00001", &["SYN-1"])
                ]
            }))
        }),
    );
    let (bio, server) = cmp_serve(app).await;
    let result = bio
        .call("search_models", &json!({"query": "SYN"}))
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["query"], "SYN");
    assert_eq!(result["returned"], 2);
    assert_eq!(result["total"], 2);
    assert_eq!(result["truncated"], false);
    assert_eq!(result["models"][0]["model_id"], "SIDM00001");
    assert_eq!(result["models"][1]["model_id"], "SIDM00002");
    assert!(result.to_string().contains("SYN-1"));
    assert!(!result.to_string().contains("SIDG00001"));
}

#[tokio::test]
async fn search_genes_exact_eq_versus_ilike() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = captured.clone();
    let app = Router::new().route(
        "/genes",
        get(move |uri: Uri| {
            let seen = seen.clone();
            async move {
                let query = uri.query().unwrap_or("").to_string();
                seen.lock().unwrap().push(query.clone());
                let params = decode_query(&query);
                let filter = params.get("filter").cloned().unwrap_or_default();
                let genes = vec![
                    cmp_gene("SIDG00001", "SYN1"),
                    cmp_gene("SIDG00002", "SYN12"),
                ];
                let matched: Vec<Value> = if filter.contains("\"op\":\"eq\"") {
                    genes
                        .into_iter()
                        .filter(|gene| gene["attributes"]["symbol"] == "SYN1")
                        .collect()
                } else {
                    genes
                };
                axum::Json(jsonapi_list(matched.clone(), matched.len()))
            }
        }),
    );
    let (bio, server) = cmp_serve(app).await;
    let exact = bio
        .call("search_genes", &json!({"query": "SYN1", "exact": true}))
        .await
        .unwrap();
    let fuzzy = bio
        .call("search_genes", &json!({"query": "SYN1", "exact": false}))
        .await
        .unwrap();
    server.abort();
    let queries = captured.lock().unwrap().clone();
    let filters: Vec<String> = queries
        .iter()
        .map(|q| decode_query(q).get("filter").cloned().unwrap_or_default())
        .collect();
    assert!(
        filters
            .iter()
            .any(|f| f.contains("\"op\":\"eq\"") && !f.contains('%')),
        "{filters:?}"
    );
    assert!(
        filters
            .iter()
            .any(|f| f.contains("\"op\":\"ilike\"") && f.contains("%SYN1%")),
        "{filters:?}"
    );
    assert_eq!(exact["exact"], true);
    assert_eq!(exact["returned"], 1);
    assert_eq!(exact["genes"][0]["gene_id"], "SIDG00001");
    assert_eq!(exact["genes"][0]["symbol"], "SYN1");
    assert_eq!(fuzzy["returned"], 2);
    assert_eq!(fuzzy["genes"][0]["gene_id"], "SIDG00001");
    assert_eq!(fuzzy["genes"][1]["gene_id"], "SIDG00002");
    assert_eq!(fuzzy["truncated"], false);
}

#[tokio::test]
async fn gene_dependencies_uses_gene_scoped_path_and_model_filter() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let seen = captured.clone();
    let app = Router::new()
        .route(
            "/genes",
            get({
                let seen = seen.clone();
                move |uri: Uri| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(format!(
                            "{}?{}",
                            uri.path(),
                            uri.query().unwrap_or("")
                        ));
                        axum::Json(jsonapi_list(vec![cmp_gene("SIDG00001", "SYN1")], 1))
                    }
                }
            }),
        )
        .route(
            "/genes/{gene_id}/datasets/crispr_ko",
            get({
                let seen = seen.clone();
                move |Path(gene_id): Path<String>, uri: Uri| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(format!(
                            "{}?{}",
                            uri.path(),
                            uri.query().unwrap_or("")
                        ));
                        assert_eq!(gene_id, "SIDG00001");
                        let params = decode_query(uri.query().unwrap_or(""));
                        let mut rows = vec![
                            crispr_row("volatile-b", "SIDM00002", "Sanger", 2.0),
                            crispr_row("volatile-a", "SIDM00001", "Broad", 1.5),
                            crispr_row("volatile-c", "SIDM00001", "Sanger", 1.2),
                        ];
                        if let Some(filter) = params.get("filter") {
                            if filter.contains("SIDM00001") {
                                rows.retain(|row| {
                                    row["relationships"]["model"]["data"]["id"] == "SIDM00001"
                                });
                            }
                        }
                        let total = rows.len();
                        axum::Json(jsonapi_list(rows, total))
                    }
                }
            }),
        )
        .route(
            "/datasets/crispr_ko",
            get({
                let seen = seen.clone();
                move |uri: Uri| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(uri.path().to_string());
                        StatusCode::INTERNAL_SERVER_ERROR
                    }
                }
            }),
        );
    let (bio, server) = cmp_serve(app).await;
    let all = bio
        .call(
            "gene_dependencies",
            &json!({"gene_symbol": "SYN1", "max_records": 50}),
        )
        .await
        .unwrap();
    let filtered = bio
        .call(
            "gene_dependencies",
            &json!({"gene_symbol": "SYN1", "model_id": "sidm00001"}),
        )
        .await
        .unwrap();
    server.abort();
    let traffic = captured.lock().unwrap().clone();
    assert!(
        traffic
            .iter()
            .any(|row| row.starts_with("/genes/SIDG00001/datasets/crispr_ko")),
        "{traffic:?}"
    );
    assert!(
        traffic
            .iter()
            .all(|row| row != "/datasets/crispr_ko" && !row.starts_with("/datasets/crispr_ko?")),
        "{traffic:?}"
    );
    let model_filter = traffic
        .iter()
        .find(|row| row.contains("crispr_ko") && row.contains("filter="))
        .map(|row| decode_query(row.split_once('?').map(|(_, q)| q).unwrap_or("")))
        .and_then(|q| q.get("filter").cloned())
        .expect("model filter");
    assert!(model_filter.contains("\"op\":\"has\""), "{model_filter}");
    assert!(model_filter.contains("SIDM00001"), "{model_filter}");
    assert_eq!(all["gene"]["gene_id"], "SIDG00001");
    assert_eq!(all["gene"]["symbol"], "SYN1");
    assert_eq!(all["total"], 3);
    assert_eq!(all["returned"], 3);
    assert_eq!(all["truncated"], false);
    assert_eq!(all["dependencies"][0]["model_id"], "SIDM00001");
    assert_eq!(all["dependencies"][0]["source"], "Broad");
    assert_eq!(all["dependencies"][1]["model_id"], "SIDM00001");
    assert_eq!(all["dependencies"][1]["source"], "Sanger");
    assert_eq!(all["dependencies"][0]["bf"], 1.5);
    assert_eq!(all["dependencies"][0]["fc_clean_qn"], "-0.3");
    assert!(!all.to_string().contains("volatile-"));
    assert!(all["dependencies"][0].get("id").is_none());
    assert_eq!(filtered["returned"], 2);
    assert_eq!(filtered["model_id"], "SIDM00001");
}

#[tokio::test]
async fn cmp_jsonapi_errors_and_http_429_do_not_echo_secrets() {
    for (status, body, expected) in [
        (
            StatusCode::OK,
            json!({"errors": [{"detail": "secret-token", "status": "400"}]}).to_string(),
            "error document",
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            "secret-token".into(),
            "HTTP 429",
        ),
    ] {
        let app = Router::new().route(
            "/models",
            get({
                let body = body.clone();
                move || {
                    let body = body.clone();
                    async move { (status, body).into_response() }
                }
            }),
        );
        let (bio, server) = cmp_serve(app).await;
        let error = bio
            .call("list_models", &json!({"tissue": "Lung"}))
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
