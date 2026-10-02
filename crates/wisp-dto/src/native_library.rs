//! Native library commands. The store is app-global, so these calls do not
//! take a project id and are announced apart from the settings allowlist.
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "wisp.native-library.v1";

pub const COMMANDS: &[&str] = &["native_library_search", "native_library_delete"];

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteRequest {
    pub id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_and_delete_fixtures_round_trip_without_a_project_id() {
        let search: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-library/v1/search.json"
        ))
        .unwrap();
        assert_eq!(search["schema"], SCHEMA);
        assert!(search["project_id"].is_null());
        assert!(COMMANDS.contains(&search["command"].as_str().unwrap()));
        let request: SearchRequest = serde_json::from_value(search["args"].clone()).unwrap();
        assert_eq!(request.query, "RNA");
        assert_eq!(request.kind.as_deref(), Some("code"));
        let rows: Vec<crate::LibraryItemSummary> =
            serde_json::from_value(search["result"].clone()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "item-1");
        assert_eq!(rows[0].kind, "code");
        assert_eq!(rows[0].source_project_id, "research-1");
        assert!(rows[0].code_preview.contains("pandas"));
        let encoded = serde_json::to_value(&rows[0]).unwrap();
        assert_eq!(encoded["code_preview"], rows[0].code_preview);
        let mut extra = search["args"].clone();
        extra["active_window"] = serde_json::json!("main");
        assert!(serde_json::from_value::<SearchRequest>(extra).is_err());

        let delete: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-library/v1/delete.json"
        ))
        .unwrap();
        assert!(delete["project_id"].is_null());
        assert!(COMMANDS.contains(&delete["command"].as_str().unwrap()));
        let removal: DeleteRequest = serde_json::from_value(delete["args"].clone()).unwrap();
        assert_eq!(removal.id, rows[0].id);
        assert_eq!(delete["result"], true);
    }
}
