//! `write` — overwrite a file, sandboxed to the project root.

use crate::env::{ToolEnv, ToolResult};
use crate::tool::{arg_str, Tool};
use async_trait::async_trait;
use serde_json::json;
use wisp_llm::ToolSchema;

pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "write",
            "Write content to a file, overwriting if it exists. The path must be inside the project root.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to write" },
                    "content": { "type": "string", "description": "Content to write to the file" }
                },
                "required": ["path", "content"]
            }),
        )
    }
    fn preview(&self, args: &serde_json::Value) -> String {
        arg_str(args, "path").unwrap_or_default()
    }
    async fn run(&self, args: &serde_json::Value, env: &dyn ToolEnv) -> ToolResult {
        let path = match arg_str(args, "path") {
            Ok(p) => p,
            Err(e) => return ToolResult::fail(e),
        };
        let content = match arg_str(args, "content") {
            Ok(c) => c,
            Err(e) => return ToolResult::fail(e),
        };
        let real = match crate::safety::validate_file_path(env.project_root(), &path) {
            Ok(p) => p,
            Err(e) => return ToolResult::fail(format!("write {path} error: {e}")),
        };
        if let Some(parent) = real.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return ToolResult::fail(format!(
                    "write {path} error: cannot create parent dir: {e}"
                ));
            }
        }
        if let Err(e) = std::fs::write(&real, &content) {
            return ToolResult::fail(format!("write {path} error: {e}"));
        }
        ToolResult::ok(format!("write {} bytes to {} ok", content.len(), path))
    }
}
