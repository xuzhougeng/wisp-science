//! Native research journey. The call names one project and reads only that
//! project's mainline history.
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "wisp.native-journey.v1";

pub const COMMANDS: &[&str] = &["native_research_journey"];

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JourneyRequest {
    pub from: i64,
    pub until: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journey_fixture_requires_a_project_id() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-journey/v1/range.json"
        ))
        .unwrap();
        assert_eq!(fixture["schema"], SCHEMA);
        assert_eq!(fixture["project_id"], "research-1");
        assert!(COMMANDS.contains(&fixture["command"].as_str().unwrap()));
        let request: JourneyRequest = serde_json::from_value(fixture["args"].clone()).unwrap();
        assert!(request.from < request.until);
        let page: crate::ResearchJourney =
            serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(page.entries[0].title, "a finding");
        assert!(!page.truncated);
        let mut extra = fixture["args"].clone();
        extra["active_window"] = serde_json::json!("main");
        assert!(serde_json::from_value::<JourneyRequest>(extra).is_err());
    }
}
