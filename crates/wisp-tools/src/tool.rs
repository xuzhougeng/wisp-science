//! The `Tool` trait every built-in or MCP-backed tool implements.

use crate::env::{Approval, ToolEnv, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use wisp_llm::ToolSchema;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn schema(&self) -> ToolSchema;
    /// Keep this tool callable but omit its schema from ordinary model requests.
    /// Deferred tools are discovered through the registry's MCP search/dispatch pair.
    fn defer_schema(&self) -> bool {
        false
    }
    /// Minimum approval required even when the host has no persisted rule for
    /// this tool. Third-party plugin tools use `Ask`; built-ins default to
    /// `Allow`. An explicit host `Deny` always wins.
    fn minimum_approval(&self) -> Approval {
        Approval::Allow
    }
    /// One-line preview shown in the tool-call card (e.g. the file path).
    fn preview(&self, _args: &Value) -> String {
        String::new()
    }
    /// Hook fired before `run` (e.g. `edit` emits a unified diff here).
    async fn before(&self, _args: &Value, _env: &dyn ToolEnv) {}
    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult;
}

/// Pull a string argument, or fail with a clear message.
pub fn arg_str(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| format!("missing required argument '{}'", key))
}

/// Pull an optional string argument.
pub fn arg_str_opt(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(String::from)
}

/// Pull an optional integer argument.
pub fn arg_int_opt(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(|v| v.as_i64())
}

/// Pull an optional bool argument.
pub fn arg_bool_opt(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(|v| v.as_bool())
}
