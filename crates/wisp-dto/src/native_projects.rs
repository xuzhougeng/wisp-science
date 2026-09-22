//! Native project commands. Payloads stay on the settings transport, but this
//! family is announced separately so a client can see project writes without
//! treating them as settings edits.
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "wisp.native-projects.v1";

pub const COMMANDS: &[&str] = &["native_project_create"];

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateProjectRequest {
    pub name: String,
    pub workspace_dir: String,
    pub description: String,
    pub agent_context: String,
    pub standard_layout: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_fixture_round_trips_without_a_project_id() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-projects/v1/create.json"
        ))
        .unwrap();
        assert_eq!(fixture["schema"], SCHEMA);
        assert!(fixture["project_id"].is_null());
        assert!(COMMANDS.contains(&fixture["command"].as_str().unwrap()));
        let request: CreateProjectRequest =
            serde_json::from_value(fixture["args"].clone()).unwrap();
        assert_eq!(request.name, "RNA-seq 研究");
        assert!(!request.standard_layout);
        assert!(request.agent_context.contains("raw data"));
        let summary: crate::ProjectSummary =
            serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(summary.id, "research-1");
        assert_eq!(summary.workspace_dir, request.workspace_dir);
        assert_eq!(summary.session_count, 0);
        let mut extra = fixture["args"].clone();
        extra["active_window"] = serde_json::json!("main");
        assert!(serde_json::from_value::<CreateProjectRequest>(extra).is_err());
    }
}
