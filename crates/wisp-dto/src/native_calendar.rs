//! Native research calendar. The query names its projects in the body and does
//! not take a settings project id, so it cannot retarget the WebView.
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "wisp.native-calendar.v1";

pub const COMMANDS: &[&str] = &["native_research_calendar"];

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarRequest {
    pub project_ids: Vec<String>,
    pub from: i64,
    pub until: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_fixture_round_trips_without_a_project_id() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-calendar/v1/month.json"
        ))
        .unwrap();
        assert_eq!(fixture["schema"], SCHEMA);
        assert!(fixture["project_id"].is_null());
        assert!(COMMANDS.contains(&fixture["command"].as_str().unwrap()));
        let request: CalendarRequest = serde_json::from_value(fixture["args"].clone()).unwrap();
        assert_eq!(request.project_ids, ["research-1"]);
        assert!(request.from < request.until);
        assert!(request.until - request.from <= 32 * 86400);
        let rows: Vec<crate::ResearchCalendarProject> =
            serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].project_id, "research-1");
        assert!(rows[0].error.is_none());
        assert_eq!(rows[0].history.entries[0].title, "a finding");
        assert!(!rows[0].history.truncated);
        let mut extra = fixture["args"].clone();
        extra["active_window"] = serde_json::json!("main");
        assert!(serde_json::from_value::<CalendarRequest>(extra).is_err());
    }
}
