use serde::{Deserialize, Serialize};

/// A fresh preview is required before a conversation operation touches files.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SessionArtifactPreview {
    pub fingerprint: String,
    pub artifacts: Vec<String>,
    pub files: Vec<String>,
    pub retained: Vec<RetainedSessionArtifact>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RetainedSessionArtifact {
    pub name: String,
    /// Stable localization key suffix (shared, upload, changed, unavailable).
    pub reason: String,
}
