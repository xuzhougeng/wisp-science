//! Native publication workspace. Reads and creates belong to one explicit project.
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "wisp.native-publication.v1";

pub const COMMANDS: &[&str] = &["native_publication_workspace", "native_publication_create"];

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRequest {
    #[serde(default)]
    pub publication_id: Option<String>,
    #[serde(default)]
    pub revision_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub revision_label: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct NativePublication {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct NativePublicationRevision {
    pub id: String,
    pub publication_id: String,
    pub revision_number: i64,
    pub label: String,
    pub state: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct NativePublicationItem {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub ordinal: i64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct NativePublicationWorkspace {
    pub publications: Vec<NativePublication>,
    pub publication: Option<NativePublication>,
    pub revision: Option<NativePublicationRevision>,
    pub items: Vec<NativePublicationItem>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_fixture_requires_a_project_id() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-publication/v1/workspace.json"
        ))
        .unwrap();
        assert_eq!(fixture["schema"], SCHEMA);
        assert_eq!(fixture["project_id"], "research-1");
        assert!(COMMANDS.contains(&fixture["command"].as_str().unwrap()));
        let request: WorkspaceRequest = serde_json::from_value(fixture["args"].clone()).unwrap();
        assert!(request.publication_id.is_none());
        let page: NativePublicationWorkspace =
            serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(page.publications[0].title, "RNA-seq paper");
        assert_eq!(page.publication.as_ref().unwrap().project_id, "research-1");
        assert_eq!(page.revision.as_ref().unwrap().label, "v1");
        assert_eq!(page.items[0].kind, "claim");
        let mut extra = fixture["args"].clone();
        extra["active_window"] = serde_json::json!("main");
        assert!(serde_json::from_value::<WorkspaceRequest>(extra).is_err());
    }
}
