//! Literal keyword matches in a project's persisted conversation history.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProjectChatSearchCursor {
    pub ts: i64,
    pub session_id: String,
    pub message_seq: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProjectChatSearchHit {
    pub session_id: String,
    pub session_title: String,
    pub message_seq: i64,
    pub role: String,
    pub ts: i64,
    /// Plain text around the first match; rendered as text, never HTML.
    pub before: String,
    pub matched: String,
    pub after: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProjectChatSearchPage {
    pub items: Vec<ProjectChatSearchHit>,
    pub next_cursor: Option<ProjectChatSearchCursor>,
}

/// Resolved only when opening a hit; search pages never load whole outlines.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProjectChatSearchTarget {
    pub user_index: usize,
    pub before_seq: Option<i64>,
}
