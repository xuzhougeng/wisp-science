use serde::{Deserialize, Serialize};

/// A fresh preview is required before a conversation operation touches files.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SessionArtifactPreview {
    pub fingerprint: String,
    pub artifacts: Vec<String>,
    pub files: Vec<String>,
    pub retained: Vec<RetainedSessionArtifact>,
}

/// Artifact previews for deleting a conversation together with its branches
/// and subagents. References between them keep nothing, so each preview is
/// only valid for this deletion.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SessionFamilyArtifactPreview {
    pub id: String,
    pub preview: SessionArtifactPreview,
    /// Its branches and subagents, nested ones included, deepest first.
    pub children: Vec<SessionChildArtifactPreview>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SessionChildArtifactPreview {
    pub id: String,
    pub preview: SessionArtifactPreview,
}

/// Conversations nested under one that is about to be deleted, nested ones
/// included. Deleting it can take them along or leave them standalone.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SessionChildrenPreview {
    pub subagents: usize,
    pub branches: usize,
    /// Explorations in the open rounds these conversations started.
    pub explorations: usize,
}

impl SessionChildrenPreview {
    pub fn is_empty(&self) -> bool {
        self.subagents + self.branches + self.explorations == 0
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RetainedSessionArtifact {
    pub name: String,
    /// Stable localization key suffix (shared, upload, changed, unavailable).
    pub reason: String,
}
