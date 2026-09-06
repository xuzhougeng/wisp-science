use super::*;
use crate::http::{Http, MAX_RESPONSE};
use axum::{
    extract::Query,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

fn test_bio(base: &str) -> NativeBio {
    let base = base.trim_end_matches('/');
    NativeBio::test_client(
        &[
            ("CIVIC_GRAPHQL_URL".into(), format!("{base}/api/graphql")),
            ("CLINGEN_SEARCH_URL".into(), base.into()),
            ("CLINGEN_ACTIONABILITY_URL".into(), base.into()),
            ("CLINGEN_EREPO_URL".into(), format!("{base}/evrepo/api")),
            (
                "OPEN_TARGETS_GRAPHQL_URL".into(),
                format!("{base}/api/v4/graphql"),
            ),
            ("CIVIC_API_KEY".into(), "civic-secret-token".into()),
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

fn civic_gene() -> Value {
    json!({
        "id": 101,
        "name": "SYNTHG",
        "entrezId": 999999,
        "fullName": "synthetic gene",
        "featureAliases": ["SG"],
        "description": "Invented CIViC gene.",
        "link": "/genes/101"
    })
}

fn civic_variant() -> Value {
    json!({
        "id": 202,
        "name": "V999",
        "link": "/variants/202",
        "variantAliases": ["p.Val999Glu"],
        "variantTypes": [{"id": 1, "name": "substitution", "soid": "SO:0001583"}],
        "feature": {"id": 101, "name": "SYNTHG"},
        "singleVariantMolecularProfileId": 303
    })
}

fn fixture_router() -> Router {
    Router::new()
        .route("/api/graphql", post(civic_graphql))
        .route("/api/v4/graphql", post(open_targets_graphql))
        .route("/api/validity", get(validity_table))
        .route("/api/dosage", get(dosage_table))
        .route("/ac/{ctx}/api/summ", get(actionability_table))
        .route("/evrepo/api/classifications", get(erepo_table))
}

async fn civic_graphql(headers: HeaderMap, body: String) -> impl IntoResponse {
    let auth = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let payload: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let query = payload["query"].as_str().unwrap_or("");
    let vars = &payload["variables"];
    if auth != "Bearer civic-secret-token" {
        return (
            StatusCode::UNAUTHORIZED,
            json!({"errors": [{"message": "missing key"}]}).to_string(),
        )
            .into_response();
    }
    let data = if query.contains("CivicGenes") {
        json!({"genes": conn(vec![civic_gene()], 1, false)})
    } else if query.contains("CivicGeneVariants") || query.contains("CivicVariants") {
        json!({"variants": conn(vec![civic_variant()], 3, true)})
    } else if query.contains("CivicGet") && query.contains("variant(") {
        if vars["id"] == 202 {
            json!({"variant": civic_variant()})
        } else {
            json!({"variant": Value::Null})
        }
    } else if query.contains("CivicGet") && query.contains("evidenceItem(") {
        json!({"evidenceItem": {
            "id": 1409, "name": "EID1409", "status": "ACCEPTED", "evidenceLevel": "A",
            "evidenceType": "PREDICTIVE", "evidenceDirection": "SUPPORTS",
            "significance": "SENSITIVITYRESPONSE", "link": "/evidence/1409",
            "disease": {"id": 1, "name": "synthetic melanoma", "doid": "DOID:000"},
            "therapies": [{"id": 9, "name": "synthnib", "ncitId": "C0"}],
            "molecularProfile": {"id": 303, "name": "SYNTHG V999"}
        }})
    } else if query.contains("CivicGet") && query.contains("assertion(") {
        json!({"assertion": {
            "id": 7, "name": "AID7", "status": "ACCEPTED", "assertionType": "PREDICTIVE",
            "ampLevel": "TIER_I_LEVEL_A", "link": "/assertions/7",
            "molecularProfile": {"id": 303, "name": "SYNTHG V999"}
        }})
    } else if query.contains("CivicGet") && query.contains("molecularProfile(") {
        json!({"molecularProfile": {
            "id": 303, "name": "SYNTHG V999", "rawName": "SYNTHG V999",
            "link": "/molecular-profiles/303", "isComplex": false,
            "variants": [{"id": 202, "name": "V999", "feature": {"id": 101, "name": "SYNTHG"}}]
        }})
    } else if query.contains("CivicEvidenceItems") {
        json!({"evidenceItems": conn(vec![json!({
            "id": 1409, "name": "EID1409", "status": "ACCEPTED", "evidenceLevel": "A",
            "link": "/evidence/1409"
        })], 1, false)})
    } else if query.contains("CivicAssertions") {
        json!({"assertions": conn(vec![json!({
            "id": 7, "name": "AID7", "ampLevel": "TIER_I_LEVEL_A", "link": "/assertions/7"
        })], 1, false)})
    } else if query.contains("CivicDiseases") {
        json!({"diseases": conn(vec![json!({
            "id": 11, "name": "MELANOMA", "displayName": "synthetic melanoma",
            "doid": "DOID:000", "link": "/diseases/11"
        })], 1, false)})
    } else if query.contains("CivicTherapies") {
        json!({"therapies": conn(vec![json!({
            "id": 9, "name": "synthnib", "ncitId": "C0", "link": "/therapies/9"
        })], 1, false)})
    } else if query.contains("CivicMolecularProfiles") {
        json!({"molecularProfiles": conn(vec![json!({
            "id": 303, "name": "SYNTHG V999", "link": "/molecular-profiles/303"
        })], 1, false)})
    } else {
        return (
            StatusCode::OK,
            json!({"errors": [{"message": "unknown operation"}]}).to_string(),
        )
            .into_response();
    };
    axum::Json(json!({"data": data})).into_response()
}

async fn open_targets_graphql(body: String) -> impl IntoResponse {
    let payload: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let query = payload["query"].as_str().unwrap_or("");
    let vars = &payload["variables"];
    let data = if query.contains("DiseaseDrugs") {
        if vars["id"] == "MONDO_9999999" {
            json!({"disease": Value::Null})
        } else {
            json!({"disease": {
                "id": "MONDO_0004992",
                "name": "synthetic cancer",
                "drugAndClinicalCandidates": {
                    "count": 2,
                    "rows": [
                        {"id": "CHEMBL000001", "maxClinicalStage": 4, "drug": {"id": "CHEMBL000001", "name": "synthnib", "drugType": "Small molecule"}},
                        {"id": "CHEMBL000002", "maxClinicalStage": 2, "drug": {"id": "CHEMBL000002", "name": "placebonib", "drugType": "Small molecule"}}
                    ]
                }
            }})
        }
    } else if query.contains("DiseaseTargets") {
        json!({"disease": {
            "id": "MONDO_0004992",
            "name": "synthetic cancer",
            "associatedTargets": {
                "count": 8,
                "rows": [{"score": 0.91, "target": {"id": "ENSG00000000001", "approvedSymbol": "SYNTHG"}}]
            }
        }})
    } else if query.contains("DrugDetails") {
        json!({"drug": {
            "id": "CHEMBL000001",
            "name": "synthnib",
            "drugType": "Small molecule",
            "maximumClinicalStage": 4,
            "mechanismsOfAction": {"rows": [{"mechanismOfAction": "inhibitor", "actionType": "INHIBITOR",
                "targets": [{"id": "ENSG00000000001", "approvedSymbol": "SYNTHG"}]}]}
        }})
    } else {
        json!({"ok": true, "echo": vars})
    };
    axum::Json(json!({"data": data}))
}

async fn validity_table() -> impl IntoResponse {
    axum::Json(json!({
        "total": 2,
        "rows": [
            {
                "symbol": "SYNTHG", "hgnc_id": "HGNC:999999", "disease_name": "synthetic disorder",
                "mondo": "MONDO:9999999", "moi": "AD", "sop": "SOP9", "classification": "Definitive",
                "ep": "Synthetic Panel", "perm_id": "CGGV:assertion_synth-1", "animal_model_only": false
            },
            {
                "symbol": "OTHERG", "hgnc_id": "HGNC:1", "disease_name": "other disorder",
                "mondo": "MONDO:1", "moi": "AR", "sop": "SOP9", "classification": "Limited",
                "ep": "Other Panel", "perm_id": "CGGV:assertion_other-1"
            }
        ]
    }))
}

async fn dosage_table() -> impl IntoResponse {
    axum::Json(json!({
        "total": 2,
        "rows": [
            {
                "type": 0, "symbol": "SYNTHG", "hgnc_id": "HGNC:999999", "location": "1q21",
                "haplo_assertion": "3", "triplo_assertion": "0"
            },
            {
                "type": 1, "symbol": "ISCA-37390", "hgnc_id": "ISCA-37390",
                "haplo_assertion": "40: Dosage sensitivity unlikely", "triplo_assertion": "Not yet evaluated"
            }
        ]
    }))
}

async fn actionability_table() -> impl IntoResponse {
    axum::Json(json!({
        "columns": ["docId", "geneOrVariant", "disease", "outcome", "intervention", "severity", "likelihood", "effectiveness", "natureOfIntervention", "overall", "context"],
        "rows": [
            ["DOC-1", "SYNTHG,OTHERG", "synthetic disorder", "cancer", "surveillance", 3, 3, 3, 2, 11, "Adult"],
            ["DOC-2", "OTHERG", "other", "none", "none", 0, 0, 0, 0, 0, "Adult"]
        ]
    }))
}

async fn erepo_table(Query(params): Query<HashMap<String, String>>) -> impl IntoResponse {
    let gene = params.get("gene").cloned().unwrap_or_default();
    axum::Json(json!({
        "total": 1,
        "variantInterpretations": [{
            "@id": "https://erepo.clinicalgenome.org/evrepo/api/interpretation/synth-uuid",
            "uuid": "synth-uuid",
            "caid": "CA999999",
            "variationId": "999999",
            "gene": {"label": gene, "NCBI_id": "999999"},
            "condition": {"@id": "MONDO:9999999", "label": "synthetic disorder"},
            "hgvs": ["NM_000000.1:c.1A>G"],
            "publishedDate": "2020-01-01",
            "guidelines": [{"label": "ACMG", "@id": "g1", "outcome": {"label": "Pathogenic"}}]
        }]
    }))
}

fn conn(nodes: Vec<Value>, total: u64, has_more: bool) -> Value {
    json!({
        "totalCount": total,
        "pageInfo": {"hasNextPage": has_more},
        "nodes": nodes
    })
}

#[test]
fn catalog_registers_clinical_genomics_tools() {
    let names: Vec<_> = catalog()
        .into_iter()
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        names,
        vec![
            ("clinical-genomics", "civic_gene_variants".into()),
            ("clinical-genomics", "civic_get_assertion".into()),
            ("clinical-genomics", "civic_get_evidence_item".into()),
            ("clinical-genomics", "civic_get_molecular_profile".into()),
            ("clinical-genomics", "civic_get_variant".into()),
            ("clinical-genomics", "civic_search_assertions".into()),
            ("clinical-genomics", "civic_search_diseases".into()),
            ("clinical-genomics", "civic_search_evidence".into()),
            ("clinical-genomics", "civic_search_genes".into()),
            (
                "clinical-genomics",
                "civic_search_molecular_profiles".into()
            ),
            ("clinical-genomics", "civic_search_therapies".into()),
            ("clinical-genomics", "civic_search_variants".into()),
            ("clinical-genomics", "clingen_actionability".into()),
            ("clinical-genomics", "clingen_dosage_sensitivity".into()),
            ("clinical-genomics", "clingen_gene_validity".into()),
            (
                "clinical-genomics",
                "clingen_variant_classifications".into()
            ),
            ("clinical-genomics", "open_targets_disease_drugs".into()),
            ("clinical-genomics", "open_targets_disease_targets".into()),
            ("clinical-genomics", "open_targets_drug".into()),
            ("clinical-genomics", "open_targets_graphql".into()),
        ]
    );
    assert!(crate::contains_tool("civic_search_genes"));
    assert_eq!(
        crate::domain_for_tool("civic_search_genes"),
        Some("clinical-genomics")
    );
    assert!(crate::package_selects(
        "mcp_clinical_genomics",
        "clinical-genomics"
    ));
    assert!(crate::selected_by_package("mcp_clinical_genomics"));
}

#[test]
fn rejects_unbounded_or_malformed_arguments() {
    assert!(serde_json::from_value::<civic::SearchGenes>(
        json!({"entrez_symbol": "BRAF", "api_key": "x"})
    )
    .is_err());
    assert!(require_id(0, "gene_id").is_err());
    assert!(require_id(-1, "gene_id").is_err());
    assert!(bound_page(0, CIVIC_MAX_PAGE).is_err());
    assert!(bound_page(101, CIVIC_MAX_PAGE).is_err());
    assert!(require_symbol("BRAF V600E", "gene").is_err());
    assert!(require_text(" ", "name", 8).is_err());
    assert!(require_text(&"x".repeat(MAX_TEXT + 1), "name", MAX_TEXT).is_err());
}

#[tokio::test]
async fn civic_search_and_get_report_source_urls_and_bounds() {
    let (bio, server) = serve(fixture_router()).await;
    let genes = bio
        .call(
            "civic_search_genes",
            &json!({"entrez_symbol": "SYNTHG", "max_results": 10}),
        )
        .await
        .unwrap();
    let variants = bio
        .call(
            "civic_gene_variants",
            &json!({"gene_id": 101, "max_results": 1}),
        )
        .await
        .unwrap();
    let found = bio
        .call("civic_get_variant", &json!({"variant_id": 202}))
        .await
        .unwrap();
    let missing = bio
        .call("civic_get_variant", &json!({"variant_id": 1}))
        .await
        .unwrap();
    server.abort();
    assert_eq!(genes["source"], "CIViC");
    assert_eq!(genes["source_url"], CIVIC_GRAPHQL);
    assert_eq!(genes["returned"], 1);
    assert_eq!(genes["records"][0]["url"], "https://civicdb.org/genes/101");
    assert_eq!(variants["total_count"], 3);
    assert_eq!(variants["returned"], 1);
    assert_eq!(variants["truncated"], true);
    assert_eq!(found["found"], true);
    assert_eq!(found["record"]["url"], "https://civicdb.org/variants/202");
    assert_eq!(missing["found"], false);
    assert!(missing["record"].is_null());
    assert!(!genes.to_string().contains("civic-secret-token"));
}

#[tokio::test]
async fn remaining_tools_dispatch_through_native_bio_call() {
    let (bio, server) = serve(fixture_router()).await;
    let evidence = bio
        .call(
            "civic_search_evidence",
            &json!({"disease_name": "melanoma", "evidence_level": "A", "status": "ACCEPTED"}),
        )
        .await
        .unwrap();
    let assertions = bio
        .call(
            "civic_search_assertions",
            &json!({"assertion_type": "PREDICTIVE", "amp_level": "TIER_I_LEVEL_A"}),
        )
        .await
        .unwrap();
    let diseases = bio
        .call("civic_search_diseases", &json!({"name": "melanoma"}))
        .await
        .unwrap();
    let therapies = bio
        .call("civic_search_therapies", &json!({"name": "synthnib"}))
        .await
        .unwrap();
    let profiles = bio
        .call(
            "civic_search_molecular_profiles",
            &json!({"name": "SYNTHG V999"}),
        )
        .await
        .unwrap();
    let variant_search = bio
        .call(
            "civic_search_variants",
            &json!({"name": "V999", "gene_id": 101}),
        )
        .await
        .unwrap();
    let eid = bio
        .call("civic_get_evidence_item", &json!({"evidence_id": 1409}))
        .await
        .unwrap();
    let aid = bio
        .call("civic_get_assertion", &json!({"assertion_id": 7}))
        .await
        .unwrap();
    let mp = bio
        .call("civic_get_molecular_profile", &json!({"mp_id": 303}))
        .await
        .unwrap();
    let validity = bio
        .call("clingen_gene_validity", &json!({"gene": "synthg"}))
        .await
        .unwrap();
    let dosage = bio
        .call(
            "clingen_dosage_sensitivity",
            &json!({"gene": "ISCA-37390", "include_regions": true}),
        )
        .await
        .unwrap();
    let action = bio
        .call(
            "clingen_actionability",
            &json!({"gene": "SYNTHG", "context": "adult"}),
        )
        .await
        .unwrap();
    let erepo = bio
        .call(
            "clingen_variant_classifications",
            &json!({"gene": "SYNTHG"}),
        )
        .await
        .unwrap();
    let drugs = bio
        .call(
            "open_targets_disease_drugs",
            &json!({"efo_id": "MONDO:0004992", "size": 1}),
        )
        .await
        .unwrap();
    let targets = bio
        .call(
            "open_targets_disease_targets",
            &json!({"efo_id": "MONDO_0004992"}),
        )
        .await
        .unwrap();
    let drug = bio
        .call("open_targets_drug", &json!({"chembl_id": "CHEMBL000001"}))
        .await
        .unwrap();
    let gql = bio
        .call(
            "open_targets_graphql",
            &json!({"query": "query { meta { name } }", "variables": {"id": "x"}}),
        )
        .await
        .unwrap();
    let missing_ot = bio
        .call(
            "open_targets_disease_drugs",
            &json!({"efo_id": "MONDO_9999999"}),
        )
        .await
        .unwrap();
    server.abort();
    assert_eq!(evidence["records"][0]["id"], 1409);
    assert_eq!(assertions["records"][0]["ampLevel"], "TIER_I_LEVEL_A");
    assert_eq!(diseases["records"][0]["doid"], "DOID:000");
    assert_eq!(therapies["records"][0]["ncitId"], "C0");
    assert_eq!(profiles["records"][0]["id"], 303);
    assert_eq!(variant_search["truncated"], true);
    assert_eq!(eid["found"], true);
    assert_eq!(aid["record"]["url"], "https://civicdb.org/assertions/7");
    assert_eq!(mp["record"]["id"], 303);
    assert_eq!(validity["returned"], 1);
    assert_eq!(validity["records"][0]["gene_symbol"], "SYNTHG");
    assert_eq!(
        validity["records"][0]["url"],
        "https://search.clinicalgenome.org/kb/gene-validity/CGGV:assertion_synth-1"
    );
    assert_eq!(dosage["records"][0]["haploinsufficiency"]["code"], "40");
    assert_eq!(dosage["records"][0]["record_type"], "region");
    assert_eq!(action["adult"]["returned"], 1);
    assert_eq!(
        action["adult"]["records"][0]["genes"],
        json!(["SYNTHG", "OTHERG"])
    );
    assert_eq!(erepo["records"][0]["caid"], "CA999999");
    assert_eq!(drugs["found"], true);
    assert_eq!(
        drugs["record"]["drugAndClinicalCandidates"]["rows"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(targets["record"]["associatedTargets"]["count"], 8);
    assert_eq!(drug["record"]["name"], "synthnib");
    assert_eq!(gql["source"], "Open Targets Platform");
    assert_eq!(gql["data"]["ok"], true);
    assert_eq!(missing_ot["found"], false);
}

#[tokio::test]
async fn rejects_rate_limits_malformed_json_and_graphql_errors_without_echoing_secrets() {
    for (status, body, expected) in [
        (
            StatusCode::TOO_MANY_REQUESTS,
            "civic-secret-token".into(),
            "HTTP 429",
        ),
        (StatusCode::OK, "{not-json".into(), "invalid JSON"),
        (
            StatusCode::OK,
            json!({"errors": [{"message": "Unknown field civic-secret-token"}]}).to_string(),
            "GraphQL query was rejected",
        ),
    ] {
        let app = Router::new().route(
            "/api/graphql",
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
            .call("civic_search_genes", &json!({"entrez_symbol": "SYNTHG"}))
            .await
            .unwrap_err()
            .to_string();
        server.abort();
        assert!(
            error.contains(expected),
            "{error} did not contain {expected}"
        );
        assert!(!error.contains("civic-secret-token"), "{error}");
    }
}

#[tokio::test]
async fn retries_bounded_429_then_succeeds() {
    let hits = Arc::new(StdMutex::new(0u32));
    let count = hits.clone();
    let app = Router::new().route(
        "/api/graphql",
        post(move |headers: HeaderMap, _body: String| {
            let count = count.clone();
            async move {
                assert_eq!(
                    headers.get("authorization").unwrap(),
                    "Bearer civic-secret-token"
                );
                let mut n = count.lock().unwrap();
                *n += 1;
                if *n == 1 {
                    (
                        StatusCode::TOO_MANY_REQUESTS,
                        [("retry-after", "0")],
                        "secret",
                    )
                        .into_response()
                } else {
                    axum::Json(json!({"data": {"genes": conn(vec![civic_gene()], 1, false)}}))
                        .into_response()
                }
            }
        }),
    );
    let (bio, server) = serve(app).await;
    let result = bio
        .call("civic_search_genes", &json!({"entrez_symbol": "SYNTHG"}))
        .await
        .unwrap();
    server.abort();
    assert_eq!(result["returned"], 1);
    assert!(*hits.lock().unwrap() >= 2);
}

#[tokio::test]
async fn clingen_429_and_count_mismatch_are_errors() {
    let app = Router::new().route(
        "/api/validity",
        get(|| async {
            (
                StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", "60")],
                "civic-secret-token",
            )
                .into_response()
        }),
    );
    let (bio, server) = serve(app).await;
    let error = bio
        .call("clingen_gene_validity", &json!({"gene": "SYNTHG"}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("HTTP 429"), "{error}");
    assert!(!error.contains("civic-secret-token"), "{error}");

    let app = Router::new().route(
        "/api/validity",
        get(|| async { axum::Json(json!({"total": 9, "rows": [{}]})) }),
    );
    let (bio, server) = serve(app).await;
    let error = bio
        .call("clingen_gene_validity", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("did not match"), "{error}");
}

#[tokio::test]
async fn oversized_graphql_response_is_rejected() {
    let app = Router::new().route(
        "/api/graphql",
        post(|| async { (StatusCode::OK, " ".repeat(MAX_RESPONSE + 1)).into_response() }),
    );
    let (bio, server) = serve(app).await;
    let error = bio
        .call("civic_search_genes", &json!({"entrez_symbol": "SYNTHG"}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("exceeded 4 MiB"), "{error}");
}

#[tokio::test]
async fn unknown_tool_and_read_only_graphql_guards() {
    let (bio, server) = serve(fixture_router()).await;
    let error = bio
        .call("civic_not_a_tool", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    let mutation = bio
        .call(
            "open_targets_graphql",
            &json!({"query": "mutation { delete }"}),
        )
        .await
        .unwrap_err()
        .to_string();
    let both = bio
        .call(
            "clingen_variant_classifications",
            &json!({"gene": "SYNTHG", "caid": "CA1"}),
        )
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert!(error.contains("unknown native biological tool"));
    assert!(mutation.contains("read-only"));
    assert!(both.contains("exactly one"));
}
