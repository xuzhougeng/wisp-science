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
fn catalog_registers_six_cbioportal_tools_on_cancer_models_slug() {
    let names: Vec<_> = catalog()
        .into_iter()
        .map(|(domain, schema)| (domain, schema.function.name))
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
        ]
    );
    assert!(crate::contains_tool("cbioportal_list_studies"));
    assert_eq!(
        crate::domain_for_tool("cbioportal_list_studies"),
        Some("cancer-models")
    );
    assert!(crate::package_selects("mcp_cancer_models", "cancer-models"));
    assert!(crate::selected_by_package("mcp_cancer_models"));
    assert!(!crate::contains_tool("list_models"));
    assert!(!crate::contains_tool("get_model"));
    assert!(!crate::contains_tool("search_models"));
    assert!(!crate::contains_tool("search_genes"));
    assert!(!crate::contains_tool("gene_dependencies"));
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
