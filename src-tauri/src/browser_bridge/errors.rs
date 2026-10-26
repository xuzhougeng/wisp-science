use serde_json::{json, Value};

#[allow(dead_code)]
pub const TAB_BUSY: &str = "TAB_BUSY";
pub const EXTENSION_STALE: &str = "EXTENSION_STALE";
pub const SESSION_REQUIRED: &str = "SESSION_REQUIRED";
pub const USER_CONTROLLING: &str = "USER_CONTROLLING";
pub const ASSET_BLOCKED: &str = "ASSET_BLOCKED";
pub const WORKSPACE_EXTENSION_BLOCKED: &str = "WORKSPACE_EXTENSION_BLOCKED";
pub const TAB_OUT_OF_SCOPE: &str = "TAB_OUT_OF_SCOPE";

pub fn structured(code: &str, message: &str, retryable: bool) -> String {
    json!({
        "code": code,
        "message": message,
        "retryable": retryable
    })
    .to_string()
}

pub fn from_value(error: Option<&Value>) -> String {
    match error {
        Some(Value::String(error)) => error.clone(),
        Some(error) => serde_json::to_string_pretty(error).unwrap_or_else(|_| error.to_string()),
        None => "browser extension returned an unknown error".into(),
    }
}
