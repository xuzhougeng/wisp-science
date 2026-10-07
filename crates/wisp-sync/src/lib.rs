//! End-to-end encrypted, manual snapshot sync for Wisp projects, plus the
//! remote web access rendezvous.
//!
//! The relay only stores opaque content-addressed blobs and immutable revision
//! descriptors, and only forwards opaque remote frames. Project keys and remote
//! access codes never leave clients.

mod crypto;
mod http;
mod protocol;
mod relay;
mod remote;

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
pub use remote::{
    open_frame, relay_base, remote_endpoints, seal_frame, HostFrame, RemoteCode, CLIENT_TO_HOST,
    HOST_OFFLINE_CLOSE, HOST_TO_CLIENT, REMOTE_HOST_MAX_FRAME_BYTES,
};
