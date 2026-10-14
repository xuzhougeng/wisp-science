//! Offline research checklist replication. Mutations only edit plan data;
//! synchronizing never dispatches work or starts an agent.
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "wisp.research-checklist.v1";
pub const COMMAND: &str = "native_research_checklist_sync";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SyncRequest {
    pub epoch: Option<String>,
    #[serde(default)]
    pub cursor: i64,
    pub visibility: Option<String>,
    #[serde(default)]
    pub changes: Vec<Change>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub mutation_id: String,
    pub id: String,
    /// Zero creates a new UUID. Existing records use their last observed revision.
    pub base_revision: i64,
    pub title: String,
    pub day: String,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub day: String,
    pub status: String,
    pub project_id: Option<String>,
    pub session_id: Option<String>,
    pub revision: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Outcome {
    pub mutation_id: String,
    pub id: String,
    /// `applied`, `conflict`, or `unavailable`. Unavailable never includes content.
    pub status: String,
    pub item: Option<Item>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SyncResponse {
    pub schema: String,
    pub epoch: String,
    pub cursor: i64,
    pub visibility: String,
    pub visible_project_ids: Vec<String>,
    /// A new/restored database requires a pull before any mutations are applied.
    pub reset: bool,
    pub has_more: bool,
    pub items: Vec<Item>,
    pub outcomes: Vec<Outcome>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_checklist_contract_preserves_revisions_and_rejects_dispatch_fields() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/research-checklist/v1/sync.json"
        ))
        .unwrap();
        assert_eq!(fixture["schema"], SCHEMA);
        assert_eq!(fixture["command"], COMMAND);
        let request: SyncRequest = serde_json::from_value(fixture["args"].clone()).unwrap();
        let reply: SyncResponse = serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(reply.items[0].id, request.changes[0].id);
        assert_eq!(
            reply.outcomes[0].mutation_id,
            request.changes[0].mutation_id
        );
        assert_eq!(serde_json::to_value(reply).unwrap(), fixture["result"]);
        let mut extra = fixture["args"].clone();
        extra["changes"][0]["session_id"] = serde_json::json!("dispatch");
        assert!(serde_json::from_value::<SyncRequest>(extra).is_err());
    }
}
