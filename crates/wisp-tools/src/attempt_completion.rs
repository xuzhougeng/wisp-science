//! `attempt_completion` — signal the task is done and present the final result.

use crate::env::{ToolEnv, ToolResult};
use crate::tool::{arg_str, Tool};
use async_trait::async_trait;
use serde_json::json;
use wisp_llm::ToolSchema;

pub struct AttemptCompletionTool;

#[async_trait]
impl Tool for AttemptCompletionTool {
    fn name(&self) -> &str {
        "attempt_completion"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "attempt_completion",
            "Indicate that the task is complete and provide the final result/answer to the user.",
            json!({
                "type": "object",
                "properties": {
                    "result": { "type": "string", "description": "The final result or summary of the completed task" }
                },
                "required": ["result"]
            }),
        )
    }
    async fn run(&self, args: &serde_json::Value, _env: &dyn ToolEnv) -> ToolResult {
        let result = match arg_str(args, "result") {
            Ok(r) => r,
            Err(e) => return ToolResult::fail(e),
        };
        ToolResult::ok(result)
    }
}
