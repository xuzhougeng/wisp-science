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
