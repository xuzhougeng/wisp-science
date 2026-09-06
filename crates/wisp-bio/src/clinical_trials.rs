//! Native ClinicalTrials.gov domain, independently implemented from:
//! - REST API v2 OpenAPI 2.0.5 (`https://clinicaltrials.gov/api/oas/v2`)
//! - Interactive docs (`https://clinicaltrials.gov/data-api/api`)
//! - Study data structure (`https://clinicaltrials.gov/data-api/about-api/study-data-structure`)
//! - Essie search syntax (`https://clinicaltrials.gov/find-studies/constructing-complex-search-queries`)
//!
//! References reviewed 2026-09-06. No API key is required. Tests use invented records.

#[cfg(test)]
mod tests;

use crate::http::Source;
use crate::NativeBio;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use wisp_llm::ToolSchema;

const DOMAIN: &str = "clinical-trials";
const API_ROOT: &str = "https://clinicaltrials.gov/api/v2";
const SITE_ROOT: &str = "https://clinicaltrials.gov";
const SOURCE: Source = Source("ClinicalTrials.gov", Duration::from_millis(200));
const MAX_PAGE: usize = 1000;
const MAX_TEXT: usize = 512;
const MAX_QUERY: usize = 2048;
const MAX_LOCATIONS: usize = 100;
const MAX_ENDPOINTS: usize = 200;
const MAX_INVESTIGATORS: usize = 250;
const SEARCH_FIELDS: &str = "NCTId|OfficialTitle|BriefTitle|OverallStatus|Phase|StudyType|Condition|InterventionName|LeadSponsorName|EnrollmentCount|StartDate|PrimaryCompletionDate|ContactsLocationsModule";
const DETAILS_FIELDS: &str = "protocolSection|hasResults";
const INVESTIGATOR_FIELDS: &str =
    "NCTId|BriefTitle|Condition|ContactsLocationsModule|ResponsiblePartyInvestigatorFullName|ResponsiblePartyInvestigatorAffiliation|ResponsiblePartyType";
const ENDPOINT_FIELDS: &str = "NCTId|protocolSection.outcomesModule";

const STATUSES: &[&str] = &[
    "ACTIVE_NOT_RECRUITING",
    "COMPLETED",
    "ENROLLING_BY_INVITATION",
    "NOT_YET_RECRUITING",
    "RECRUITING",
    "SUSPENDED",
    "TERMINATED",
    "WITHDRAWN",
    "AVAILABLE",
    "NO_LONGER_AVAILABLE",
    "TEMPORARILY_NOT_AVAILABLE",
    "APPROVED_FOR_MARKETING",
    "WITHHELD",
    "UNKNOWN",
];
const PHASES: &[&str] = &["NA", "EARLY_PHASE1", "PHASE1", "PHASE2", "PHASE3", "PHASE4"];
const STUDY_TYPES: &[&str] = &["INTERVENTIONAL", "OBSERVATIONAL", "EXPANDED_ACCESS"];
const SEXES: &[&str] = &["ALL", "MALE", "FEMALE"];

#[cfg(test)]
thread_local! {
    static TEST_API_ROOT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn api_root() -> String {
    #[cfg(test)]
    {
        if let Some(root) = TEST_API_ROOT.with(|slot| slot.borrow().clone()) {
            return root;
        }
    }
    API_ROOT.to_string()
}

#[cfg(test)]
#[must_use]
struct TestRootGuard;

#[cfg(test)]
impl Drop for TestRootGuard {
    fn drop(&mut self) {
        TEST_API_ROOT.with(|slot| *slot.borrow_mut() = None);
    }
}

#[cfg(test)]
fn install_test_api_root(root: String) -> TestRootGuard {
    TEST_API_ROOT.with(|slot| *slot.borrow_mut() = Some(root));
    TestRootGuard
}

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        tool(
            "search_trials",
            "Search ClinicalTrials.gov (API v2) for registered studies. Filter by condition, intervention, sponsor, location, phase, status or study type. Returns a bounded page of NCT IDs with titles, status, phase and public study URLs. Set count_total to include the upstream match count. Use get_trial_details for a full protocol record.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "condition": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT, "description": "Condition or disease query (query.cond, Essie)."},
                    "intervention": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT, "description": "Intervention or treatment query (query.intr, Essie)."},
                    "sponsor": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT, "description": "Sponsor or collaborator query (query.spons, Essie)."},
                    "location": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT, "description": "Location query (query.locn, Essie)."},
                    "status": status_schema(),
                    "phase": phase_schema(),
                    "study_type": {"type": "string", "enum": STUDY_TYPES},
                    "advanced_query": {"type": "string", "minLength": 1, "maxLength": MAX_QUERY, "description": "Essie expression merged into filter.advanced."},
                    "page_size": {"type": "integer", "minimum": 1, "maximum": MAX_PAGE, "default": 10},
                    "page_token": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT},
                    "count_total": {"type": "boolean", "default": false}
                }
            }),
        ),
        tool(
            "get_trial_details",
            "Retrieve one ClinicalTrials.gov study by NCT ID. Returns protocol metadata, eligibility, endpoints, a bounded location list and the public study URL. Unknown identifiers are reported as not found rather than empty evidence. NCT numbers are NCT followed by eight digits.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["nct_id"],
                "properties": {
                    "nct_id": {"type": "string", "minLength": 1, "maxLength": 32, "description": "NCT identifier, optionally without the NCT prefix."}
                }
            }),
        ),
        tool(
            "search_by_eligibility",
            "Find ClinicalTrials.gov studies whose eligibility constraints match a patient profile. Age filters use MinimumAge/MaximumAge RANGE expressions; sex matching includes all-comers studies. Defaults to recruiting studies. Provide at least one of condition, eligibility_keywords, min_age, max_age or sex. Returns a bounded page of NCT IDs and study URLs.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "condition": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT},
                    "eligibility_keywords": {"type": "string", "minLength": 1, "maxLength": MAX_QUERY, "description": "Phrase matched in EligibilityCriteria."},
                    "min_age": {"type": "string", "minLength": 1, "maxLength": 32, "description": "Patient age lower bound, e.g. '18 Years'. Matches studies whose MinimumAge is at most this value."},
                    "max_age": {"type": "string", "minLength": 1, "maxLength": 32, "description": "Patient age upper bound, e.g. '70 Years'. Matches studies whose MaximumAge is at least this value."},
                    "sex": {"type": "string", "enum": SEXES},
                    "status": status_schema(),
                    "page_size": {"type": "integer", "minimum": 1, "maximum": MAX_PAGE, "default": 10},
                    "page_token": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT}
                }
            }),
        ),
        tool(
            "search_by_sponsor",
            "Find ClinicalTrials.gov studies by lead sponsor name (query.lead / LeadSponsorName). Optionally filter by condition, phase and status. Returns a bounded page of NCT IDs, titles and public study URLs.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["sponsor_name"],
                "properties": {
                    "sponsor_name": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT},
                    "condition": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT},
                    "status": status_schema(),
                    "phase": phase_schema(),
                    "page_size": {"type": "integer", "minimum": 1, "maximum": MAX_PAGE, "default": 10},
                    "page_token": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT},
                    "count_total": {"type": "boolean", "default": false}
                }
            }),
        ),
        tool(
            "search_investigators",
            "List investigators and site contacts from ClinicalTrials.gov studies matching an investigator name, institution, location, condition or status. Includes overall officials, responsible-party investigators and location contacts, each with the related NCT ID and study URL. Provide at least one search constraint.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "investigator_name": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT},
                    "condition": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT},
                    "institution": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT, "description": "LocationFacility phrase. Takes precedence over location."},
                    "location": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT},
                    "status": status_schema(),
                    "page_size": {"type": "integer", "minimum": 1, "maximum": MAX_PAGE, "default": 20}
                }
            }),
        ),
        tool(
            "analyze_endpoints",
            "Summarize protocol outcome measures from ClinicalTrials.gov. Provide nct_id for a single study, or a condition to aggregate a bounded page of studies (optionally filtered by phase and start date). nct_id takes precedence when both are set. Returns primary, secondary and other endpoints plus the most common measures.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "nct_id": {"type": "string", "minLength": 1, "maxLength": 32},
                    "condition": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT},
                    "phase": phase_schema(),
                    "start_date_after": {"type": "string", "description": "Inclusive StartDate lower bound: YYYY, YYYY-MM or YYYY-MM-DD."},
                    "page_size": {"type": "integer", "minimum": 1, "maximum": MAX_PAGE, "default": 50}
                }
            }),
        ),
    ]
}

fn tool(name: &str, description: &str, parameters: Value) -> (&'static str, ToolSchema) {
    (DOMAIN, ToolSchema::new(name, description, parameters))
}

fn status_schema() -> Value {
    json!({
        "oneOf": [
            {"type": "string", "enum": STATUSES},
            {"type": "array", "minItems": 1, "maxItems": STATUSES.len(), "items": {"type": "string", "enum": STATUSES}}
        ]
    })
}

fn phase_schema() -> Value {
    json!({
        "oneOf": [
            {"type": "string", "enum": PHASES},
            {"type": "array", "minItems": 1, "maxItems": PHASES.len(), "items": {"type": "string", "enum": PHASES}}
        ]
    })
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(45), dispatch(bio, name, args))
        .await
        .map_err(|_| anyhow!("ClinicalTrials.gov request exceeded 45 seconds"))?
}

async fn dispatch(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "search_trials" => {
            let args: SearchTrials = parse_args(args, "search_trials")?;
            let page = studies_page(
                bio,
                search_trials_params(&args)?,
                args.page_size,
                args.page_token.as_deref(),
                args.count_total,
                SEARCH_FIELDS,
            )
            .await?;
            search_result(&page, args.count_total)
        }
        "get_trial_details" => {
            let args: TrialId = parse_args(args, "get_trial_details")?;
            let nct = normalize_nct(&args.nct_id)?;
            match fetch_study(bio, &nct, DETAILS_FIELDS).await? {
                Some(study) => details_result(&study, &nct),
                None => Ok(not_found(&nct)),
            }
        }
        "search_by_eligibility" => {
            let args: SearchEligibility = parse_args(args, "search_by_eligibility")?;
            let page = studies_page(
                bio,
                eligibility_params(&args)?,
                args.page_size,
                args.page_token.as_deref(),
                false,
                SEARCH_FIELDS,
            )
            .await?;
            search_result(&page, false)
        }
        "search_by_sponsor" => {
            let args: SearchSponsor = parse_args(args, "search_by_sponsor")?;
            let page = studies_page(
                bio,
                sponsor_params(&args)?,
                args.page_size,
                args.page_token.as_deref(),
                args.count_total,
                SEARCH_FIELDS,
            )
            .await?;
            search_result(&page, args.count_total)
        }
        "search_investigators" => {
            let args: SearchInvestigators = parse_args(args, "search_investigators")?;
            let page = studies_page(
                bio,
                investigator_params(&args)?,
                args.page_size,
                None,
                false,
                INVESTIGATOR_FIELDS,
            )
            .await?;
            investigators_result(&page)
        }
        "analyze_endpoints" => {
            let args: AnalyzeEndpoints = parse_args(args, "analyze_endpoints")?;
            let nct = optional_text(&args.nct_id, "nct_id", 32)?;
            let condition = optional_text(&args.condition, "condition", MAX_TEXT)?;
            if nct.is_none() && condition.is_none() {
                bail!("analyze_endpoints requires nct_id or condition");
            }
            if let Some(nct_id) = nct {
                let nct_id = normalize_nct(&nct_id)?;
                let study = fetch_study(bio, &nct_id, ENDPOINT_FIELDS)
                    .await?
                    .with_context(|| format!("ClinicalTrials.gov did not find {nct_id}"))?;
                return endpoints_result(std::slice::from_ref(&study), Some(&nct_id), None);
            }
            let page = studies_page(
                bio,
                endpoints_params(condition.as_deref().unwrap(), &args)?,
                args.page_size,
                None,
                false,
                ENDPOINT_FIELDS,
            )
            .await?;
            let studies = studies_array(&page)?;
            endpoints_result(studies, None, condition.as_deref())
        }
        _ => bail!("unknown native biological tool: {name}"),
    }
}

fn parse_args<T: for<'de> Deserialize<'de>>(args: &Value, name: &str) -> Result<T> {
    serde_json::from_value(args.clone()).with_context(|| format!("invalid {name} arguments"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchTrials {
    condition: Option<String>,
    intervention: Option<String>,
    sponsor: Option<String>,
    location: Option<String>,
    status: Option<OneOrMany>,
    phase: Option<OneOrMany>,
    study_type: Option<String>,
    advanced_query: Option<String>,
    #[serde(default = "default_search_page")]
    page_size: usize,
    page_token: Option<String>,
    #[serde(default)]
    count_total: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrialId {
    nct_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchEligibility {
    condition: Option<String>,
    eligibility_keywords: Option<String>,
    min_age: Option<String>,
    max_age: Option<String>,
    sex: Option<String>,
    status: Option<OneOrMany>,
    #[serde(default = "default_search_page")]
    page_size: usize,
    page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchSponsor {
    sponsor_name: String,
    condition: Option<String>,
    status: Option<OneOrMany>,
    phase: Option<OneOrMany>,
    #[serde(default = "default_search_page")]
    page_size: usize,
    page_token: Option<String>,
    #[serde(default)]
    count_total: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchInvestigators {
    investigator_name: Option<String>,
    condition: Option<String>,
    institution: Option<String>,
    location: Option<String>,
    status: Option<OneOrMany>,
    #[serde(default = "default_investigator_page")]
    page_size: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnalyzeEndpoints {
    nct_id: Option<String>,
    condition: Option<String>,
    phase: Option<OneOrMany>,
    start_date_after: Option<String>,
    #[serde(default = "default_endpoint_page")]
    page_size: usize,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

fn default_search_page() -> usize {
    10
}
fn default_investigator_page() -> usize {
    20
}
fn default_endpoint_page() -> usize {
    50
}

fn search_trials_params(args: &SearchTrials) -> Result<Vec<(String, String)>> {
    check_page(args.page_size, args.page_token.as_deref())?;
    let mut params = Vec::new();
    push_query(
        &mut params,
        "query.cond",
        optional_text(&args.condition, "condition", MAX_TEXT)?,
    );
    push_query(
        &mut params,
        "query.intr",
        optional_text(&args.intervention, "intervention", MAX_TEXT)?,
    );
    push_query(
        &mut params,
        "query.spons",
        optional_text(&args.sponsor, "sponsor", MAX_TEXT)?,
    );
    push_query(
        &mut params,
        "query.locn",
        optional_text(&args.location, "location", MAX_TEXT)?,
    );
    push_status(&mut params, enum_list(&args.status, STATUSES, "status")?)?;
    let mut advanced = Vec::new();
    if let Some(expr) = phase_expr(&args.phase)? {
        advanced.push(expr);
    }
    if let Some(study_type) = optional_text(&args.study_type, "study_type", 32)? {
        let study_type = require_enum(&study_type, STUDY_TYPES, "study_type")?;
        advanced.push(area_term("StudyType", &study_type));
    }
    if let Some(user) = optional_text(&args.advanced_query, "advanced_query", MAX_QUERY)? {
        advanced.push(user);
    }
    push_advanced(&mut params, &advanced);
    Ok(params)
}

fn eligibility_params(args: &SearchEligibility) -> Result<Vec<(String, String)>> {
    check_page(args.page_size, args.page_token.as_deref())?;
    let condition = optional_text(&args.condition, "condition", MAX_TEXT)?;
    let keywords = optional_text(
        &args.eligibility_keywords,
        "eligibility_keywords",
        MAX_QUERY,
    )?;
    let min_age = optional_age(&args.min_age, "min_age")?;
    let max_age = optional_age(&args.max_age, "max_age")?;
    let sex = match optional_text(&args.sex, "sex", 16)? {
        Some(value) => Some(require_enum(&value, SEXES, "sex")?),
        None => None,
    };
    if condition.is_none()
        && keywords.is_none()
        && min_age.is_none()
        && max_age.is_none()
        && sex.is_none()
    {
        bail!("search_by_eligibility requires condition, eligibility_keywords, min_age, max_age or sex");
    }
    let mut params = Vec::new();
    push_query(&mut params, "query.cond", condition);
    let mut advanced = Vec::new();
    if let Some(keywords) = keywords {
        advanced.push(area_phrase("EligibilityCriteria", &keywords));
    }
    if let Some(min_age) = min_age {
        advanced.push(area_range("MinimumAge", None, Some(&min_age)));
    }
    if let Some(max_age) = max_age {
        advanced.push(area_range("MaximumAge", Some(&max_age), None));
    }
    if let Some(sex) = sex {
        if sex == "ALL" {
            advanced.push(area_term("Sex", "ALL"));
        } else {
            advanced.push(or_join(&[area_term("Sex", &sex), area_term("Sex", "ALL")]));
        }
    }
    push_advanced(&mut params, &advanced);
    let status = enum_list(&args.status, STATUSES, "status")?;
    let status = if status.is_empty() {
        vec!["RECRUITING".into()]
    } else {
        status
    };
    push_status(&mut params, status)?;
    Ok(params)
}

fn sponsor_params(args: &SearchSponsor) -> Result<Vec<(String, String)>> {
    check_page(args.page_size, args.page_token.as_deref())?;
    let sponsor = require_text(&args.sponsor_name, "sponsor_name", MAX_TEXT)?;
    let mut params = vec![("query.lead".into(), quote_phrase(sponsor))];
    push_query(
        &mut params,
        "query.cond",
        optional_text(&args.condition, "condition", MAX_TEXT)?,
    );
    push_status(&mut params, enum_list(&args.status, STATUSES, "status")?)?;
    if let Some(expr) = phase_expr(&args.phase)? {
        params.push(("filter.advanced".into(), expr));
    }
    Ok(params)
}

fn investigator_params(args: &SearchInvestigators) -> Result<Vec<(String, String)>> {
    check_page(args.page_size, None)?;
    let name = optional_text(&args.investigator_name, "investigator_name", MAX_TEXT)?;
    let condition = optional_text(&args.condition, "condition", MAX_TEXT)?;
    let institution = optional_text(&args.institution, "institution", MAX_TEXT)?;
    let location = optional_text(&args.location, "location", MAX_TEXT)?;
    let status = enum_list(&args.status, STATUSES, "status")?;
    if name.is_none()
        && condition.is_none()
        && institution.is_none()
        && location.is_none()
        && status.is_empty()
    {
        bail!("search_investigators requires investigator_name, institution, location, condition or status");
    }
    let mut params = Vec::new();
    push_query(&mut params, "query.cond", condition);
    if institution.is_none() {
        push_query(&mut params, "query.locn", location);
    }
    let mut advanced = Vec::new();
    if let Some(name) = name {
        advanced.push(or_join(&[
            area_phrase("OverallOfficialName", &name),
            area_phrase("ResponsiblePartyInvestigatorFullName", &name),
        ]));
    }
    if let Some(institution) = institution {
        advanced.push(area_phrase("LocationFacility", &institution));
    }
    push_advanced(&mut params, &advanced);
    push_status(&mut params, status)?;
    Ok(params)
}

fn endpoints_params(condition: &str, args: &AnalyzeEndpoints) -> Result<Vec<(String, String)>> {
    check_page(args.page_size, None)?;
    let mut params = vec![("query.cond".into(), condition.to_string())];
    let mut advanced = Vec::new();
    if let Some(expr) = phase_expr(&args.phase)? {
        advanced.push(expr);
    }
    if let Some(start) = optional_text(&args.start_date_after, "start_date_after", 16)? {
        if !valid_partial_date(&start) {
            bail!("start_date_after must be YYYY, YYYY-MM or YYYY-MM-DD");
        }
        advanced.push(area_range("StartDate", Some(&start), None));
    }
    push_advanced(&mut params, &advanced);
    Ok(params)
}

fn phase_expr(phase: &Option<OneOrMany>) -> Result<Option<String>> {
    let phases = enum_list(phase, PHASES, "phase")?;
    if phases.is_empty() {
        return Ok(None);
    }
    Ok(Some(or_join(
        &phases
            .iter()
            .map(|phase| area_term("Phase", phase))
            .collect::<Vec<_>>(),
    )))
}

fn check_page(page_size: usize, page_token: Option<&str>) -> Result<()> {
    if !(1..=MAX_PAGE).contains(&page_size) {
        bail!("page_size must be between 1 and {MAX_PAGE}");
    }
    if let Some(token) = page_token {
        if token.trim().is_empty() || token.len() > MAX_TEXT {
            bail!("page_token must contain 1 to {MAX_TEXT} characters");
        }
    }
    Ok(())
}

fn push_query(params: &mut Vec<(String, String)>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        params.push((key.into(), value));
    }
}

fn push_status(params: &mut Vec<(String, String)>, status: Vec<String>) -> Result<()> {
    if !status.is_empty() {
        params.push(("filter.overallStatus".into(), status.join("|")));
    }
    Ok(())
}

fn push_advanced(params: &mut Vec<(String, String)>, parts: &[String]) {
    if let Some(expr) = and_join(parts) {
        params.push(("filter.advanced".into(), expr));
    }
}

fn quote_phrase(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

fn area_phrase(area: &str, phrase: &str) -> String {
    format!("AREA[{area}]{}", quote_phrase(phrase))
}

fn area_term(area: &str, term: &str) -> String {
    format!("AREA[{area}]{term}")
}

fn area_range(area: &str, lo: Option<&str>, hi: Option<&str>) -> String {
    format!(
        "AREA[{area}]RANGE[{}, {}]",
        lo.unwrap_or("MIN"),
        hi.unwrap_or("MAX")
    )
}

fn and_join(parts: &[String]) -> Option<String> {
    let kept: Vec<&str> = parts
        .iter()
        .map(String::as_str)
        .filter(|part| !part.is_empty())
        .collect();
    if kept.is_empty() {
        return None;
    }
    Some(
        kept.into_iter()
            .map(|expr| {
                if expr.contains(" OR ") && !(expr.starts_with('(') && expr.ends_with(')')) {
                    format!("({expr})")
                } else {
                    expr.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" AND "),
    )
}

fn or_join(parts: &[String]) -> String {
    let kept: Vec<&str> = parts
        .iter()
        .map(String::as_str)
        .filter(|part| !part.is_empty())
        .collect();
    match kept.len() {
        0 => String::new(),
        1 => kept[0].to_string(),
        _ => format!("({})", kept.join(" OR ")),
    }
}

fn optional_text(value: &Option<String>, name: &str, max: usize) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(text) if text.trim().is_empty() => Ok(None),
        Some(text) => Ok(Some(require_text(text, name, max)?.to_string())),
    }
}

fn require_text<'a>(value: &'a str, name: &str, max: usize) -> Result<&'a str> {
    let text = value.trim();
    if text.is_empty() || text.len() > max {
        bail!("{name} must contain 1 to {max} characters");
    }
    Ok(text)
}

fn optional_age(value: &Option<String>, name: &str) -> Result<Option<String>> {
    let Some(text) = optional_text(value, name, 32)? else {
        return Ok(None);
    };
    if text.contains(['[', ']', ',']) {
        bail!("{name} must not contain Essie RANGE metacharacters");
    }
    Ok(Some(text))
}

fn enum_list(value: &Option<OneOrMany>, allowed: &[&str], name: &str) -> Result<Vec<String>> {
    let raw = match value {
        None => return Ok(Vec::new()),
        Some(OneOrMany::One(item)) => vec![item.clone()],
        Some(OneOrMany::Many(items)) => items.clone(),
    };
    let mut out = Vec::new();
    for item in raw {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let item = require_enum(item, allowed, name)?;
        if !out.contains(&item) {
            out.push(item);
        }
    }
    Ok(out)
}

fn require_enum(value: &str, allowed: &[&str], name: &str) -> Result<String> {
    let value = value.trim().to_ascii_uppercase();
    if allowed.iter().any(|item| *item == value) {
        Ok(value)
    } else {
        bail!("{name} contains an unsupported value");
    }
}

fn normalize_nct(raw: &str) -> Result<String> {
    let mut value = raw.trim().to_ascii_uppercase();
    if value.is_empty() {
        bail!("nct_id must be an NCT identifier such as NCT01234567");
    }
    if !value.starts_with("NCT") {
        value = format!("NCT{value}");
    }
    let digits = &value[3..];
    if digits.is_empty()
        || digits.len() > 8
        || !digits.bytes().all(|b| b.is_ascii_digit())
        || digits.chars().all(|ch| ch == '0')
    {
        bail!("nct_id must be NCT followed by 1 to 8 digits");
    }
    let number: u32 = digits
        .parse()
        .context("nct_id must be NCT followed by 1 to 8 digits")?;
    Ok(format!("NCT{number:08}"))
}

fn valid_partial_date(value: &str) -> bool {
    let parts: Vec<_> = value.split('-').collect();
    if !(1..=3).contains(&parts.len())
        || parts[0].len() != 4
        || parts
            .iter()
            .any(|part| !part.bytes().all(|b| b.is_ascii_digit()))
        || parts.iter().skip(1).any(|part| part.len() != 2)
    {
        return false;
    }
    let year = parts[0].parse().unwrap_or(0);
    let month = parts.get(1).map_or(1, |part| part.parse().unwrap_or(0));
    let day = parts.get(2).map_or(1, |part| part.parse().unwrap_or(0));
    year > 0 && chrono::NaiveDate::from_ymd_opt(year, month, day).is_some()
}

fn study_url(nct: &str) -> String {
    format!("{SITE_ROOT}/study/{nct}")
}

async fn studies_page(
    bio: &NativeBio,
    mut params: Vec<(String, String)>,
    page_size: usize,
    page_token: Option<&str>,
    count_total: bool,
    fields: &str,
) -> Result<Value> {
    params.push(("pageSize".into(), page_size.to_string()));
    params.push(("fields".into(), fields.into()));
    if count_total && page_token.is_none() {
        params.push(("countTotal".into(), "true".into()));
    }
    if let Some(token) = page_token.map(str::trim).filter(|token| !token.is_empty()) {
        params.push(("pageToken".into(), token.to_string()));
    }
    let url = format!("{}/studies", api_root());
    bio.http()
        .send(SOURCE, Method::GET, &url, &params)
        .await?
        .json()
}

async fn fetch_study(bio: &NativeBio, nct: &str, fields: &str) -> Result<Option<Value>> {
    let url = format!("{}/studies/{nct}", api_root());
    let params = vec![("fields".into(), fields.to_string())];
    let response = bio.http().send(SOURCE, Method::GET, &url, &params).await?;
    match response.status.as_u16() {
        200 => Ok(Some(response.json()?)),
        404 => Ok(None),
        _ => {
            response.check()?;
            unreachable!("check returns only on success")
        }
    }
}

fn studies_array(page: &Value) -> Result<&[Value]> {
    page.get("studies")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .context("ClinicalTrials.gov omitted the studies list")
}

fn search_result(page: &Value, count_total: bool) -> Result<Value> {
    let studies = studies_array(page)?;
    let mut trials = Vec::new();
    let mut nct_ids = Vec::new();
    for study in studies {
        if !study.is_object() {
            bail!("ClinicalTrials.gov returned an invalid study record");
        }
        let summary = trial_summary(study)?;
        nct_ids.push(summary["nct_id"].as_str().unwrap().to_string());
        trials.push(summary);
    }
    let next = page
        .get("nextPageToken")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned);
    let total = if count_total {
        page.get("totalCount").and_then(Value::as_u64)
    } else {
        None
    };
    Ok(json!({
        "source": "ClinicalTrials.gov",
        "source_url": SITE_ROOT,
        "returned": trials.len(),
        "total": total,
        "has_more": next.is_some(),
        "next_page_token": next,
        "nct_ids": nct_ids,
        "trials": trials
    }))
}

fn trial_summary(study: &Value) -> Result<Value> {
    let ident = module(study, "identificationModule");
    let nct = ident
        .get("nctId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .context("ClinicalTrials.gov omitted an NCT identifier")?;
    let status = module(study, "statusModule");
    let design = module(study, "designModule");
    let sponsor = module(study, "sponsorCollaboratorsModule");
    let conditions = module(study, "conditionsModule");
    let arms = module(study, "armsInterventionsModule");
    let locations = module(study, "contactsLocationsModule")
        .get("locations")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    Ok(json!({
        "nct_id": nct,
        "url": study_url(nct),
        "title": text(ident, "officialTitle").or_else(|| text(ident, "briefTitle")),
        "brief_title": text(ident, "briefTitle"),
        "status": text(status, "overallStatus"),
        "phases": string_list(design.get("phases")),
        "study_type": text(design, "studyType"),
        "conditions": string_list(conditions.get("conditions")),
        "interventions": named_list(arms.get("interventions")),
        "sponsor": sponsor.get("leadSponsor").and_then(|value| text(value, "name")),
        "enrollment": design.pointer("/enrollmentInfo/count").and_then(Value::as_u64),
        "start_date": struct_date(status, "startDateStruct"),
        "primary_completion_date": struct_date(status, "primaryCompletionDateStruct"),
        "locations_count": locations
    }))
}

fn details_result(study: &Value, requested: &str) -> Result<Value> {
    if !study.is_object() {
        bail!("ClinicalTrials.gov returned an invalid study record");
    }
    let ident = module(study, "identificationModule");
    let nct = ident
        .get("nctId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(requested);
    let status = module(study, "statusModule");
    let design = module(study, "designModule");
    let sponsor = module(study, "sponsorCollaboratorsModule");
    let description = module(study, "descriptionModule");
    let eligibility = module(study, "eligibilityModule");
    let outcomes = module(study, "outcomesModule");
    let conditions = module(study, "conditionsModule");
    let arms = module(study, "armsInterventionsModule");
    let contacts = module(study, "contactsLocationsModule");
    let collaborators = named_list(sponsor.get("collaborators"));
    let locations_raw = contacts
        .get("locations")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let locations_count = locations_raw.len();
    let locations: Vec<Value> = locations_raw
        .iter()
        .take(MAX_LOCATIONS)
        .map(location_record)
        .collect();
    let healthy = eligibility
        .get("healthyVolunteers")
        .and_then(Value::as_bool);
    let (primary, primary_truncated) = outcome_list(outcomes, "primaryOutcomes", "PRIMARY");
    let (secondary, secondary_truncated) = outcome_list(outcomes, "secondaryOutcomes", "SECONDARY");
    let (other, other_truncated) = outcome_list(outcomes, "otherOutcomes", "OTHER");
    Ok(json!({
        "source": "ClinicalTrials.gov",
        "source_url": study_url(nct),
        "found": true,
        "nct_id": nct,
        "trial": {
            "nct_id": nct,
            "url": study_url(nct),
            "title": text(ident, "officialTitle").or_else(|| text(ident, "briefTitle")),
            "brief_title": text(ident, "briefTitle"),
            "acronym": text(ident, "acronym"),
            "status": text(status, "overallStatus"),
            "phases": string_list(design.get("phases")),
            "study_type": text(design, "studyType"),
            "conditions": string_list(conditions.get("conditions")),
            "interventions": named_list(arms.get("interventions")),
            "sponsor": sponsor.get("leadSponsor").and_then(|value| text(value, "name")),
            "collaborators": collaborators,
            "enrollment": design.pointer("/enrollmentInfo/count").and_then(Value::as_u64),
            "start_date": struct_date(status, "startDateStruct"),
            "primary_completion_date": struct_date(status, "primaryCompletionDateStruct"),
            "completion_date": struct_date(status, "completionDateStruct"),
            "brief_summary": text(description, "briefSummary"),
            "detailed_description": text(description, "detailedDescription"),
            "eligibility_criteria": text(eligibility, "eligibilityCriteria"),
            "minimum_age": text(eligibility, "minimumAge"),
            "maximum_age": text(eligibility, "maximumAge"),
            "sex": text(eligibility, "sex"),
            "healthy_volunteers": healthy,
            "primary_outcomes": primary,
            "secondary_outcomes": secondary,
            "other_outcomes": other,
            "locations_count": locations_count,
            "locations_returned": locations.len(),
            "locations_truncated": locations_count > MAX_LOCATIONS,
            "locations": locations,
            "has_results": study.get("hasResults").and_then(Value::as_bool),
            "outcomes_truncated": primary_truncated || secondary_truncated || other_truncated
        }
    }))
}

fn not_found(nct: &str) -> Value {
    json!({
        "source": "ClinicalTrials.gov",
        "source_url": study_url(nct),
        "found": false,
        "nct_id": nct,
        "trial": null
    })
}

fn investigators_result(page: &Value) -> Result<Value> {
    let studies = studies_array(page)?;
    let mut investigators = Vec::new();
    let mut nct_ids = Vec::new();
    let mut seen = HashSet::new();
    let mut truncated = false;
    for study in studies {
        if !study.is_object() {
            bail!("ClinicalTrials.gov returned an invalid study record");
        }
        let ident = module(study, "identificationModule");
        let nct = ident
            .get("nctId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .context("ClinicalTrials.gov omitted an NCT identifier")?;
        nct_ids.push(nct.to_string());
        let title = text(ident, "briefTitle");
        let conditions = string_list(module(study, "conditionsModule").get("conditions"));
        let contacts = module(study, "contactsLocationsModule");
        let sponsor = module(study, "sponsorCollaboratorsModule");
        for official in objects(contacts.get("overallOfficials")) {
            if !push_investigator(
                &mut investigators,
                &mut seen,
                nct,
                title.as_deref(),
                &conditions,
                text(official, "name"),
                text(official, "role"),
                text(official, "affiliation"),
                text(official, "affiliation"),
                None,
            ) {
                truncated = true;
                break;
            }
        }
        let party = sponsor
            .get("responsibleParty")
            .filter(|value| value.is_object());
        if let Some(party) = party {
            if !push_investigator(
                &mut investigators,
                &mut seen,
                nct,
                title.as_deref(),
                &conditions,
                text(party, "investigatorFullName"),
                text(party, "type").or_else(|| Some("RESPONSIBLE_PARTY".into())),
                text(party, "investigatorAffiliation"),
                text(party, "investigatorAffiliation"),
                None,
            ) {
                truncated = true;
            }
        }
        if truncated {
            break;
        }
        for location in objects(contacts.get("locations")) {
            let facility = text(location, "facility");
            let place = location_label(location);
            for contact in objects(location.get("contacts")) {
                if !push_investigator(
                    &mut investigators,
                    &mut seen,
                    nct,
                    title.as_deref(),
                    &conditions,
                    text(contact, "name"),
                    text(contact, "role"),
                    facility.clone(),
                    facility.clone(),
                    place.clone(),
                ) {
                    truncated = true;
                    break;
                }
            }
            if truncated {
                break;
            }
        }
        if truncated {
            break;
        }
    }
    Ok(json!({
        "source": "ClinicalTrials.gov",
        "source_url": SITE_ROOT,
        "returned": investigators.len(),
        "trials_analyzed": nct_ids.len(),
        "truncated": truncated,
        "nct_ids": nct_ids,
        "investigators": investigators
    }))
}

#[allow(clippy::too_many_arguments)]
fn push_investigator(
    investigators: &mut Vec<Value>,
    seen: &mut HashSet<(String, String, String)>,
    nct: &str,
    title: Option<&str>,
    conditions: &Value,
    name: Option<String>,
    role: Option<String>,
    affiliation: Option<String>,
    facility: Option<String>,
    location: Option<String>,
) -> bool {
    let Some(name) = name.filter(|value| !value.is_empty()) else {
        return true;
    };
    let role = role.unwrap_or_default();
    if !seen.insert((name.clone(), role.clone(), nct.to_string())) {
        return true;
    }
    if investigators.len() >= MAX_INVESTIGATORS {
        return false;
    }
    investigators.push(json!({
        "name": name,
        "role": if role.is_empty() { Value::Null } else { json!(role) },
        "affiliation": affiliation,
        "facility": facility,
        "location": location,
        "nct_id": nct,
        "url": study_url(nct),
        "study_title": title,
        "conditions": conditions
    }));
    true
}

fn endpoints_result(
    studies: &[Value],
    nct_id: Option<&str>,
    condition: Option<&str>,
) -> Result<Value> {
    let mut primary = Vec::new();
    let mut secondary = Vec::new();
    let mut other = Vec::new();
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut order = Vec::new();
    let mut nct_ids = Vec::new();
    let mut truncated = false;
    for study in studies {
        if !study.is_object() {
            bail!("ClinicalTrials.gov returned an invalid study record");
        }
        if let Some(nct) = module(study, "identificationModule")
            .get("nctId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            nct_ids.push(nct.to_string());
        }
        let outcomes = module(study, "outcomesModule");
        truncated |= append_endpoints(
            &mut primary,
            &mut counts,
            &mut order,
            outcomes,
            "primaryOutcomes",
            "PRIMARY",
        );
        truncated |= append_endpoints(
            &mut secondary,
            &mut counts,
            &mut order,
            outcomes,
            "secondaryOutcomes",
            "SECONDARY",
        );
        truncated |= append_endpoints(
            &mut other,
            &mut counts,
            &mut order,
            outcomes,
            "otherOutcomes",
            "OTHER",
        );
    }
    let mut ranked: Vec<(usize, usize, &String)> = order
        .iter()
        .enumerate()
        .map(|(index, measure)| (*counts.get(measure).unwrap_or(&0), index, measure))
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let common: Vec<Value> = ranked
        .into_iter()
        .take(20)
        .map(|(count, _, measure)| json!({"measure": measure, "count": count}))
        .collect();
    Ok(json!({
        "source": "ClinicalTrials.gov",
        "source_url": nct_id.map(study_url).unwrap_or_else(|| SITE_ROOT.to_string()),
        "mode": if nct_id.is_some() { "single" } else { "aggregate" },
        "nct_id": nct_id,
        "condition": condition,
        "trials_analyzed": studies.len(),
        "truncated": truncated,
        "nct_ids": nct_ids,
        "primary_endpoints": primary,
        "secondary_endpoints": secondary,
        "other_endpoints": other,
        "common_measures": common
    }))
}

fn append_endpoints(
    dest: &mut Vec<Value>,
    counts: &mut HashMap<String, usize>,
    order: &mut Vec<String>,
    outcomes: &Value,
    key: &str,
    kind: &str,
) -> bool {
    let mut truncated = false;
    for outcome in objects(outcomes.get(key)) {
        if dest.len() >= MAX_ENDPOINTS {
            truncated = true;
            break;
        }
        if let Some(measure) = text(outcome, "measure") {
            if !counts.contains_key(&measure) {
                order.push(measure.clone());
            }
            *counts.entry(measure.clone()).or_insert(0) += 1;
            dest.push(json!({
                "measure": measure,
                "time_frame": text(outcome, "timeFrame"),
                "description": text(outcome, "description"),
                "type": kind
            }));
        } else {
            dest.push(json!({
                "measure": null,
                "time_frame": text(outcome, "timeFrame"),
                "description": text(outcome, "description"),
                "type": kind
            }));
        }
    }
    truncated
}

fn outcome_list(outcomes: &Value, key: &str, kind: &str) -> (Vec<Value>, bool) {
    let mut items = Vec::new();
    let mut truncated = false;
    for outcome in objects(outcomes.get(key)) {
        if items.len() >= MAX_ENDPOINTS {
            truncated = true;
            break;
        }
        items.push(json!({
            "measure": text(outcome, "measure"),
            "time_frame": text(outcome, "timeFrame"),
            "description": text(outcome, "description"),
            "type": kind
        }));
    }
    (items, truncated)
}

fn location_record(location: &Value) -> Value {
    json!({
        "facility": text(location, "facility"),
        "city": text(location, "city"),
        "state": text(location, "state"),
        "country": text(location, "country"),
        "zip": text(location, "zip"),
        "status": text(location, "status"),
        "contacts": location.get("contacts").cloned().unwrap_or(Value::Null)
    })
}

fn location_label(location: &Value) -> Option<String> {
    let parts: Vec<&str> = ["city", "state", "country"]
        .iter()
        .filter_map(|key| location.get(*key).and_then(Value::as_str).map(str::trim))
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

fn module<'a>(study: &'a Value, name: &str) -> &'a Value {
    study
        .pointer(&format!("/protocolSection/{name}"))
        .unwrap_or(&Value::Null)
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

fn struct_date(module: &Value, key: &str) -> Option<String> {
    module
        .pointer(&format!("/{key}/date"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

fn string_list(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Null) | None => json!([]),
        Some(Value::Array(items)) => json!(items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()),
        Some(Value::String(item)) if !item.trim().is_empty() => json!([item.trim()]),
        Some(_) => json!([]),
    }
}

fn named_list(value: Option<&Value>) -> Vec<String> {
    objects(value)
        .filter_map(|item| text(item, "name"))
        .collect()
}

fn objects(value: Option<&Value>) -> impl Iterator<Item = &Value> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.is_object())
}
