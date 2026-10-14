//! Encrypted browser/native-client request, shared by desktop and server hosts.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize, Serialize)]
pub struct RemoteRequest {
    pub nonce: String,
    pub seq: u64,
    pub id: String,
    pub command: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub args: Value,
}
