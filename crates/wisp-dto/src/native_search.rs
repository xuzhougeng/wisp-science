//! Read-only, cross-project native search. The envelope project is a ranking
//! preference, never a mutation of the desktop's active project.
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "wisp.native-search.v1";
pub const COMMANDS: &[&str] = &["native_workspace_search"];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    pub query: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    Project,
    Artifact,
    Session,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SearchItem {
    pub kind: SearchKind,
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    pub title: String,
    pub detail: String,
    pub session_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SearchResponse {
    pub schema: String,
    pub query: String,
    pub preferred_project_id: Option<String>,
    pub items: Vec<SearchItem>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_search_fixture_round_trips() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-search/v1/search.json"
        ))
        .unwrap();
        let response: SearchResponse = serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(response.schema, SCHEMA);
        assert_eq!(response.items[0].kind, SearchKind::Session);
        assert_eq!(response.items[0].session_id.as_deref(), Some("session-1"));
        assert_eq!(serde_json::to_value(response).unwrap(), fixture["result"]);
        assert!(serde_json::from_value::<SearchRequest>(
            serde_json::json!({"query":"x","window":"main"})
        )
        .is_err());
    }
}
