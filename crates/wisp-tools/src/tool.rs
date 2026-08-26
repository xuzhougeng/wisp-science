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
    /// Whether this tool only retrieves — no writes, no state changes, nothing
    /// to undo. Read-only tools stay callable in plan mode on top of
    /// [`crate::PLAN_MODE_READ_ONLY`], which cannot name tools it never sees
    /// (every MCP-backed retrieval tool). Default `false`: a tool nobody
    /// classified is refused while planning.
    fn read_only(&self) -> bool {
        false
    }
    /// Whether independent calls may overlap inside one model-issued batch.
    /// This is stricter than [`Self::read_only`]: implementations must not
    /// prompt, mutate host state, materialize artifacts, or depend on another
    /// call in the same batch. Default false keeps dynamic tools sequential
    /// until their complete behavior is explicitly audited.
    fn parallel_safe(&self) -> bool {
        false
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
