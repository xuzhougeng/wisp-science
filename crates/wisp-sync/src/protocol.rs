use serde::{Deserialize, Serialize};

pub const SYNC_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceFile {
    /// Slash-separated workspace-relative path. Absolute paths and `..` are invalid.
    pub path: String,
    pub size: u64,
    /// Hash of the plaintext file, used only on devices after decrypting the manifest.
    pub sha256: String,
    /// Hash of the opaque encrypted blob stored by the relay.
    pub blob_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceManifest {
    pub version: u32,
    pub files: Vec<WorkspaceFile>,
    #[serde(default)]
    pub skipped_paths: Vec<String>,
}

impl Default for WorkspaceManifest {
    fn default() -> Self {
        Self {
            version: SYNC_PROTOCOL_VERSION,
            files: Vec::new(),
            skipped_paths: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncRevision {
    pub protocol_version: u32,
    pub project_id: String,
    pub revision_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_revision: Option<String>,
    pub device_id: String,
    pub created_at: i64,
    pub metadata_blob: String,
    pub manifest_blob: String,
    /// Opaque encrypted workspace blobs. The relay validates they exist before commit.
    #[serde(default)]
    pub workspace_blobs: Vec<String>,
    /// Hash of the portable plaintext metadata plus plaintext workspace manifest.
    pub state_hash: String,
    /// HMAC-SHA256 over this descriptor (with this field empty), keyed by the
    /// project key. The relay cannot forge or splice revision history.
    #[serde(default)]
    pub auth_tag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncHead {
    pub revision_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<String>,
    pub revision: SyncRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitOutcome {
    Committed(SyncHead),
    Conflict(Option<SyncHead>),
}
