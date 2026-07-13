//! End-to-end encrypted, manual snapshot sync for Wisp projects.
//!
//! The relay only stores opaque content-addressed blobs and immutable revision
//! descriptors. Project keys never leave clients.

mod crypto;
mod http;
mod protocol;
mod relay;

pub use crypto::{
    decrypt_blob, encrypt_blob, random_project_key, sha256_hex, sign_revision, verify_revision,
    PROJECT_KEY_BYTES,
};
pub use http::{relay_router, HttpRelay, RelayHttpState, MAX_RELAY_BODY_BYTES};
pub use protocol::{
    CommitOutcome, CommitRequest, SyncHead, SyncRevision, WorkspaceFile, WorkspaceManifest,
    SYNC_PROTOCOL_VERSION,
};
pub use relay::{FileRelay, SyncTransport};
