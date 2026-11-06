use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocxChangeKind {
    Deletion,
    Insertion,
    Rewrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocxReviewAction {
    Accept,
    Reject,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum DocxReviewSelection {
    Change(String),
    Kind(DocxChangeKind),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocxChange {
    pub id: String,
    pub paragraph_id: String,
    pub kind: DocxChangeKind,
    pub author: String,
    pub date: String,
    pub old: String,
    pub new: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocxReviewSnapshot {
    pub source_sha256: String,
    pub changes: Vec<DocxChange>,
    /// Unsupported revision elements in the main document remain untouched.
    pub unsupported_count: usize,
    /// Access is assigned by the desktop host, never by document metadata.
    pub writable: bool,
}

/// Native Files transport: explicit owner, never the hidden WebView's selection.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeDocxReviewRequest {
    pub session_id: Option<String>,
    pub path: String,
    pub source_sha256: Option<String>,
    pub selection: Option<DocxReviewSelection>,
    pub action: Option<DocxReviewAction>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NativeDocxReview {
    pub schema: String,
    pub project_id: String,
    pub session_id: Option<String>,
    pub path: String,
    pub review: DocxReviewSnapshot,
    pub content: crate::FileContent,
}
