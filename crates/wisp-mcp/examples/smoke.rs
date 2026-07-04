//! Smoke test for wisp-mcp: launches the mock MCP server via `uv` and
//! round-trips tools/list + tools/call. Run with:
//!   cargo run -p wisp-mcp --example smoke

use std::sync::Arc;
use wisp_mcp::{McpClient, McpTool};
use wisp_tools::{Tool, ToolEnv, ToolEvent, ToolResult};

struct NullEnv;
#[async_trait::async_trait]
impl ToolEnv for NullEnv {
    fn project_root(&self) -> &std::path::Path {
        std::path::Path::new(".")
    }
    async fn confirm(&self, _m: &str) -> bool {
        true
    }
    async fn emit(&self, _e: ToolEvent) {}
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mock = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("python")
        .join("mock_mcp_server.py");
    let args = vec![
        "run".into(),
        "--no-project".into(),
        "python".into(),
        mock.to_string_lossy().to_string(),
    ];
    let client = Arc::new(McpClient::launch("uv", &args).await?);

    let tools = client.tools_list().await?;
    println!("tools/list -> {} tool(s):", tools.len());
    for t in &tools {
        println!("  - {} : {}", t.name, t.description);
    }

    let env = NullEnv;
    let echo = McpTool::new(tools[0].clone(), client.clone());
    let args = serde_json::json!({ "text": "hello mcp" });
    let res = echo.run(&args, &env).await;
    println!(
        "tools/call echo -> success={} content={}",
        res.success, res.content
    );
    assert!(res.success);
    assert_eq!(res.content, "echo: hello mcp");
    let _ = ToolResult::ok(""); // touch ToolResult import
    println!("wisp-mcp smoke OK");
    Ok(())
}
