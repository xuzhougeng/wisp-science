use super::*;
use crate::http::{Http, MAX_RESPONSE};
use crate::NativeBio;
use axum::{
    extract::Path,
    http::{StatusCode, Uri},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

fn synthetic_study() -> Value {
    json!({
        "protocolSection": {
            "identificationModule": {
                "nctId": "NCT00000001",
                "briefTitle": "Synthetic diabetes trial",
                "officialTitle": "A synthetic trial of metformin in type 2 diabetes",
                "acronym": "SYNTH-DM"
            },
            "statusModule": {
                "overallStatus": "RECRUITING",
                "startDateStruct": {"date": "2024-01"},
                "primaryCompletionDateStruct": {"date": "2026-06"},
                "completionDateStruct": {"date": "2027-01"}
            },
            "designModule": {
                "studyType": "INTERVENTIONAL",
                "phases": ["PHASE3"],
                "enrollmentInfo": {"count": 120}
            },
            "sponsorCollaboratorsModule": {
                "leadSponsor": {"name": "Synthetic Pharma"},
                "collaborators": [{"name": "Example University"}],
                "responsibleParty": {
                    "type": "PRINCIPAL_INVESTIGATOR",
                    "investigatorFullName": "Jordan Lee",
                    "investigatorAffiliation": "Example University"
                }
            },
            "descriptionModule": {
                "briefSummary": "Invented summary.",
                "detailedDescription": "Invented description."
            },
            "eligibilityModule": {
                "eligibilityCriteria": "Inclusion: adults with an invented diagnosis.",
                "minimumAge": "18 Years",
                "maximumAge": "75 Years",
                "sex": "ALL",
                "healthyVolunteers": false
            },
            "outcomesModule": {
                "primaryOutcomes": [{
                    "measure": "HbA1c change",
                    "timeFrame": "26 weeks",
                    "description": "Invented primary endpoint."
                }],
                "secondaryOutcomes": [{
                    "measure": "Weight",
                    "timeFrame": "26 weeks"
                }],
                "otherOutcomes": [{"measure": "Exploratory biomarker"}]
            },
            "conditionsModule": {"conditions": ["Type 2 Diabetes Mellitus"]},
            "armsInterventionsModule": {"interventions": [{"name": "Metformin"}]},
            "contactsLocationsModule": {
                "overallOfficials": [{
                    "name": "Alex Rivera",
                    "affiliation": "Example University",
                    "role": "PRINCIPAL_INVESTIGATOR"
                }],
                "locations": [{
                    "facility": "Example Medical Center",
                    "city": "Boston",
                    "state": "Massachusetts",
                    "country": "United States",
                    "zip": "02115",
                    "status": "RECRUITING",
                    "contacts": [{"name": "Casey Nguyen", "role": "CONTACT"}]
                }]
            }
        },
        "hasResults": false
    })
}

fn paged(studies: Vec<Value>, next: Option<&str>, total: Option<u64>) -> Value {
    let mut page = json!({"studies": studies});
    if let Some(token) = next {
        page["nextPageToken"] = json!(token);
    }
    if let Some(total) = total {
        page["totalCount"] = json!(total);
    }
    page
}

fn test_bio() -> NativeBio {
    NativeBio::test_client(
        &[],
        Http(reqwest::Client::builder().no_proxy().build().unwrap()),
    )
    .unwrap()
}

async fn spawn(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), task)
}

fn attach(base: &str) -> (NativeBio, TestRootGuard) {
    let guard = install_test_api_root(format!("{base}/api/v2"));
    (test_bio(), guard)
}

fn decode(raw: &str) -> String {
    let raw = raw.replace('+', " ");
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(value) = u8::from_str_radix(hex, 16) {
                    out.push(value);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_map(raw: &str) -> HashMap<String, String> {
    decode(raw)
        .split('&')
        .filter(|part| !part.is_empty())
        .filter_map(|part| {
            part.split_once('=')
                .map(|(key, value)| (key.to_string(), value.to_string()))
        })
        .collect()
}

#[test]
fn catalog_uses_hyphenated_domain_and_six_tools() {
    let expected = [
        "search_trials",
        "get_trial_details",
        "search_by_eligibility",
        "search_by_sponsor",
        "search_investigators",
        "analyze_endpoints",
    ];
    let tools: Vec<_> = crate::catalog()
        .into_iter()
        .filter(|(domain, _)| *domain == "clinical-trials")
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        tools,
        expected
            .iter()
            .map(|name| ("clinical-trials", (*name).to_string()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        crate::domain_for_tool("search_trials"),
        Some("clinical-trials")
    );
    assert!(crate::package_selects(
        "mcp_clinical_trials",
        "clinical-trials"
    ));
    assert!(crate::package_selects("mcp_bio", "clinical-trials"));
    let named = crate::tools_for_package(
        Arc::new(crate::NativeBio::new(&[]).unwrap()),
        "mcp_clinical_trials",
    );
    assert_eq!(named.len(), 6);
}

#[test]
fn validates_identifiers_bounds_and_builds_official_query_params() {
    assert_eq!(normalize_nct("nct4567890").unwrap(), "NCT04567890");
    assert_eq!(normalize_nct("12").unwrap(), "NCT00000012");
    for invalid in ["", "NCT", "NCT00000000", "NCT123456789", "NCT12AB"] {
        assert!(normalize_nct(invalid).is_err(), "{invalid}");
    }

    let search: SearchTrials = serde_json::from_value(json!({
        "condition": "diabetes",
        "intervention": "metformin",
        "sponsor": "NIH",
        "location": "Boston",
        "status": ["RECRUITING", "NOT_YET_RECRUITING"],
        "phase": ["PHASE2", "PHASE3"],
        "study_type": "INTERVENTIONAL",
        "advanced_query": "AREA[EnrollmentCount]RANGE[100,MAX]",
        "page_size": 25,
        "count_total": true
    }))
    .unwrap();
    let params: HashMap<_, _> = search_trials_params(&search).unwrap().into_iter().collect();
    assert_eq!(params["query.cond"], "diabetes");
    assert_eq!(params["query.intr"], "metformin");
    assert_eq!(params["query.spons"], "NIH");
    assert_eq!(params["query.locn"], "Boston");
    assert_eq!(
        params["filter.overallStatus"],
        "RECRUITING|NOT_YET_RECRUITING"
    );
    assert_eq!(
        params["filter.advanced"],
        "(AREA[Phase]PHASE2 OR AREA[Phase]PHASE3) AND AREA[StudyType]INTERVENTIONAL AND AREA[EnrollmentCount]RANGE[100,MAX]"
    );

    let eligibility: SearchEligibility = serde_json::from_value(json!({
        "condition": "diabetes",
        "eligibility_keywords": "BRCA",
        "min_age": "18 Years",
        "max_age": "70 Years",
        "sex": "FEMALE"
    }))
    .unwrap();
    let params: HashMap<_, _> = eligibility_params(&eligibility)
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(params["query.cond"], "diabetes");
    assert_eq!(params["filter.overallStatus"], "RECRUITING");
    assert_eq!(
        params["filter.advanced"],
        "AREA[EligibilityCriteria]\"BRCA\" AND AREA[MinimumAge]RANGE[MIN, 18 Years] AND AREA[MaximumAge]RANGE[70 Years, MAX] AND (AREA[Sex]FEMALE OR AREA[Sex]ALL)"
    );

    let sponsor: SearchSponsor = serde_json::from_value(json!({
        "sponsor_name": "Pfizer",
        "condition": "cancer",
        "phase": "PHASE3",
        "status": "RECRUITING"
    }))
    .unwrap();
    let params: HashMap<_, _> = sponsor_params(&sponsor).unwrap().into_iter().collect();
    assert_eq!(params["query.lead"], "\"Pfizer\"");
    assert_eq!(params["query.cond"], "cancer");
    assert_eq!(params["filter.overallStatus"], "RECRUITING");
    assert_eq!(params["filter.advanced"], "AREA[Phase]PHASE3");

    let investigators: SearchInvestigators = serde_json::from_value(json!({
        "investigator_name": "Smith",
        "institution": "Mayo Clinic",
        "location": "Boston",
        "condition": "Alzheimer"
    }))
    .unwrap();
    let params: HashMap<_, _> = investigator_params(&investigators)
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(params["query.cond"], "Alzheimer");
    assert!(!params.contains_key("query.locn"));
    assert_eq!(
        params["filter.advanced"],
        "(AREA[OverallOfficialName]\"Smith\" OR AREA[ResponsiblePartyInvestigatorFullName]\"Smith\") AND AREA[LocationFacility]\"Mayo Clinic\""
    );

    let endpoints: AnalyzeEndpoints = serde_json::from_value(json!({
        "condition": "diabetes",
        "phase": ["PHASE3"],
        "start_date_after": "2020-01-01",
        "page_size": 50
    }))
    .unwrap();
    let params: HashMap<_, _> = endpoints_params("diabetes", &endpoints)
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(params["query.cond"], "diabetes");
    assert_eq!(
        params["filter.advanced"],
        "AREA[Phase]PHASE3 AND AREA[StartDate]RANGE[2020-01-01, MAX]"
    );

    assert!(serde_json::from_value::<SearchTrials>(json!({"page_size": 0})).is_ok());
    assert!(
        search_trials_params(&serde_json::from_value(json!({"page_size": 0})).unwrap()).is_err()
    );
    assert!(
        search_trials_params(&serde_json::from_value(json!({"page_size": 1001})).unwrap()).is_err()
    );
    assert!(serde_json::from_value::<SearchTrials>(json!({"api_key": "secret"})).is_err());
    assert!(eligibility_params(&serde_json::from_value(json!({})).unwrap()).is_err());
    assert!(investigator_params(&serde_json::from_value(json!({})).unwrap()).is_err());
    assert!(serde_json::from_value::<SearchTrials>(json!({"status": "NOPE"})).is_ok());
    assert!(
        search_trials_params(&serde_json::from_value(json!({"status": "NOPE"})).unwrap()).is_err()
    );
}

#[test]
fn search_mapping_keeps_nct_ids_source_urls_and_unknown_totals() {
    let page = paged(vec![synthetic_study()], Some("page-2"), Some(9));
    let result = search_result(&page, true).unwrap();
    assert_eq!(result["source"], "ClinicalTrials.gov");
    assert_eq!(result["source_url"], "https://clinicaltrials.gov");
    assert_eq!(result["nct_ids"], json!(["NCT00000001"]));
    assert_eq!(result["returned"], 1);
    assert_eq!(result["total"], 9);
    assert_eq!(result["has_more"], true);
    assert_eq!(result["next_page_token"], "page-2");
    assert_eq!(
        result["trials"][0]["url"],
        "https://clinicaltrials.gov/study/NCT00000001"
    );
    assert_eq!(result["trials"][0]["sponsor"], "Synthetic Pharma");
    assert_eq!(result["trials"][0]["phases"], json!(["PHASE3"]));

    let empty = search_result(&paged(vec![], None, Some(0)), false).unwrap();
    assert_eq!(empty["returned"], 0);
    assert_eq!(empty["total"], Value::Null);
    assert_eq!(empty["has_more"], false);
    assert!(search_result(&json!({}), false).is_err());
    assert!(search_result(&json!({"studies": [{}]}), false).is_err());
}

#[tokio::test]
async fn search_trials_uses_shipped_dispatch_against_fake_upstream() {
    let captured = Arc::new(StdMutex::new(String::new()));
    let request = captured.clone();
    let body = paged(vec![synthetic_study()], Some("page-2"), Some(3));
    let app = Router::new().route(
        "/api/v2/studies",
        get(move |uri: Uri| {
            let request = request.clone();
            let body = body.clone();
            async move {
                *request.lock().unwrap() = uri.query().unwrap_or("").to_string();
                Json(body)
            }
        }),
    );
    let (base, server) = spawn(app).await;
    let (bio, _guard) = attach(&base);
    let result = bio
        .call(
            "search_trials",
            &json!({
                "condition": "diabetes",
                "phase": "PHASE3",
                "status": "RECRUITING",
                "count_total": true,
                "page_size": 10
            }),
        )
        .await
        .unwrap();
    server.abort();
    let params = query_map(&captured.lock().unwrap());
    assert_eq!(params["query.cond"], "diabetes");
    assert_eq!(params["filter.overallStatus"], "RECRUITING");
    assert_eq!(params["filter.advanced"], "AREA[Phase]PHASE3");
    assert_eq!(params["pageSize"], "10");
    assert_eq!(params["countTotal"], "true");
    assert!(params["fields"].contains("NCTId"));
    assert_eq!(result["nct_ids"], json!(["NCT00000001"]));
    assert_eq!(result["source"], "ClinicalTrials.gov");
    assert_eq!(
        result["trials"][0]["url"],
        "https://clinicaltrials.gov/study/NCT00000001"
    );
    assert_eq!(result["total"], 3);
    assert_eq!(result["has_more"], true);
}

#[tokio::test]
async fn remaining_tools_round_trip_invented_records() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let study = synthetic_study();
    let list = paged(vec![study.clone()], None, None);
    let request = captured.clone();
    let list_body = list.clone();
    let study_body = study.clone();
    let app = Router::new()
        .route(
            "/api/v2/studies",
            get({
                let request = request.clone();
                move |uri: Uri| {
                    let request = request.clone();
                    let list_body = list_body.clone();
                    async move {
                        request
                            .lock()
                            .unwrap()
                            .push(uri.query().unwrap_or("").to_string());
                        Json(list_body)
                    }
                }
            }),
        )
        .route(
            "/api/v2/studies/{nct_id}",
            get({
                let request = request.clone();
                move |Path(nct_id): Path<String>, uri: Uri| {
                    let request = request.clone();
                    let study_body = study_body.clone();
                    async move {
                        request
                            .lock()
                            .unwrap()
                            .push(format!("{nct_id}?{}", uri.query().unwrap_or("")));
                        if nct_id == "NCT00000999" {
                            StatusCode::NOT_FOUND.into_response()
                        } else {
                            Json(study_body).into_response()
                        }
                    }
                }
            }),
        );
    let (base, server) = spawn(app).await;
    let (bio, _guard) = attach(&base);

    let details = bio
        .call("get_trial_details", &json!({"nct_id": "1"}))
        .await
        .unwrap();
    assert_eq!(details["found"], true);
    assert_eq!(details["nct_id"], "NCT00000001");
    assert_eq!(details["trial"]["healthy_volunteers"], false);
    assert_eq!(details["trial"]["primary_outcomes"][0]["type"], "PRIMARY");
    assert_eq!(
        details["source_url"],
        "https://clinicaltrials.gov/study/NCT00000001"
    );

    let missing = bio
        .call("get_trial_details", &json!({"nct_id": "NCT00000999"}))
        .await
        .unwrap();
    assert_eq!(missing["found"], false);
    assert_eq!(missing["nct_id"], "NCT00000999");
    assert!(missing["trial"].is_null());

    let eligibility = bio
        .call(
            "search_by_eligibility",
            &json!({"condition": "diabetes", "sex": "MALE"}),
        )
        .await
        .unwrap();
    assert_eq!(eligibility["nct_ids"], json!(["NCT00000001"]));

    let sponsor = bio
        .call(
            "search_by_sponsor",
            &json!({"sponsor_name": "Synthetic Pharma"}),
        )
        .await
        .unwrap();
    assert_eq!(sponsor["trials"][0]["nct_id"], "NCT00000001");

    let investigators = bio
        .call(
            "search_investigators",
            &json!({"condition": "diabetes", "page_size": 20}),
        )
        .await
        .unwrap();
    let names: Vec<_> = investigators["investigators"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"Alex Rivera"));
    assert!(names.contains(&"Jordan Lee"));
    assert!(names.contains(&"Casey Nguyen"));
    assert_eq!(
        investigators["investigators"][0]["url"],
        "https://clinicaltrials.gov/study/NCT00000001"
    );

    let single = bio
        .call("analyze_endpoints", &json!({"nct_id": "NCT00000001"}))
        .await
        .unwrap();
    assert_eq!(single["mode"], "single");
    assert_eq!(single["primary_endpoints"][0]["measure"], "HbA1c change");
    assert_eq!(single["common_measures"][0]["measure"], "HbA1c change");

    let aggregate = bio
        .call("analyze_endpoints", &json!({"condition": "diabetes"}))
        .await
        .unwrap();
    assert_eq!(aggregate["mode"], "aggregate");
    assert_eq!(aggregate["trials_analyzed"], 1);
    server.abort();

    let queries = captured.lock().unwrap();
    assert!(queries
        .iter()
        .any(|q| decode(q).contains("query.lead=\"Synthetic Pharma\"")));
    assert!(queries
        .iter()
        .any(|q| decode(q).contains("filter.overallStatus=RECRUITING")
            && decode(q).contains("AREA[Sex]MALE")));
}

#[tokio::test]
async fn rejects_upstream_errors_malformed_json_and_oversized_bodies() {
    for (status, body, expected) in [
        (StatusCode::TOO_MANY_REQUESTS, "ignored".into(), "HTTP 429"),
        (StatusCode::OK, "{".into(), "invalid JSON"),
        (
            StatusCode::OK,
            " ".repeat(MAX_RESPONSE + 1),
            "exceeded 4 MiB",
        ),
    ] {
        let app = Router::new().route(
            "/api/v2/studies",
            get(move || {
                let body = body.clone();
                async move { (status, [("retry-after", "60")], body).into_response() }
            }),
        );
        let (base, server) = spawn(app).await;
        let (bio, _guard) = attach(&base);
        let error = bio
            .call("search_trials", &json!({"condition": "synthetic"}))
            .await
            .unwrap_err()
            .to_string();
        server.abort();
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("http://"));
    }

    let error = test_bio()
        .call("not_a_tool", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown native biological tool"));
}
