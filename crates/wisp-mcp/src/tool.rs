//! `McpTool` — wraps a remote MCP tool as a `wisp_tools::Tool`.

use crate::client::{McpClient, RemoteTool};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use wisp_llm::ToolSchema;
use wisp_tools::{Tool, ToolEnv, ToolResult};

pub struct McpTool {
    name: String,
    schema: ToolSchema,
    client: Arc<McpClient>,
}

impl McpTool {
    pub fn new(tool: RemoteTool, client: Arc<McpClient>) -> Self {
        let schema = ToolSchema::new(&tool.name, &tool.description, tool.input_schema.clone());
        Self {
            name: tool.name,
            schema,
            client: Arc::clone(&client),
        }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn schema(&self) -> ToolSchema {
        self.schema.clone()
    }
    fn preview(&self, args: &Value) -> String {
        let s = args.to_string();
        s.chars().take(120).collect()
    }
    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        match self.client.tool_call(&self.name, args).await {
            Ok(content) => ToolResult::ok(if content.is_empty() {
                "(no output)".into()
            } else {
                content
            }),
            Err(e) => ToolResult::fail(format!("mcp {name} error: {e}", name = self.name)),
        }
    }
}
