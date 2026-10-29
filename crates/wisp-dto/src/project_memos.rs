//! Private project memos: notes the researcher keeps for themselves inside a
//! project. They are never injected into the agent's context; the UI offers
//! to paste a memo's body into the composer, and only a sent message reaches
//! the model.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMemo {
    pub id: String,
    pub project_id: String,
    /// May be empty; the UI then shows the first body line instead.
    pub title: String,
    pub body: String,
    pub pinned: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMemoInput {
    #[serde(default)]
    pub title: String,
    pub body: String,
}

/// Partial update; `None` leaves a field unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMemoPatch {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub pinned: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The SwiftUI 备忘录 tab sends and decodes these shapes.
    #[test]
    fn native_fixture_matches_the_memo_commands() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-settings/v1/project-memos.json"
        ))
        .unwrap();
        let memos: Vec<ProjectMemo> = serde_json::from_value(fixture["memos"].clone()).unwrap();
        assert_eq!(memos.len(), 2);
        assert!(memos[0].pinned && memos[0].title.is_empty());
        let input: ProjectMemoInput =
            serde_json::from_value(fixture["add_args"]["input"].clone()).unwrap();
        assert_eq!(input.body, "Check the doublet rate");
        let patch: ProjectMemoPatch =
            serde_json::from_value(fixture["update_args"]["patch"].clone()).unwrap();
        assert_eq!(
            patch,
            ProjectMemoPatch {
                id: "memo-titled".into(),
                pinned: Some(true),
                ..Default::default()
            }
        );
        assert_eq!(fixture["delete_args"]["memoId"], "memo-titled");
    }
}
