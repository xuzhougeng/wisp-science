//! Minimal stdio JSON-RPC MCP client + deferred `McpTool` adapter.
//!
//! Launch a server with [`McpClient::launch`], enumerate its tools with
//! [`McpClient::tools_list`], and wrap each as an [`McpTool`] to register with
//! the agent's tool registry. The registry exposes MCP schemas through a fixed
//! search/dispatch pair instead of sending the full catalog on every turn.

pub mod client;
pub mod tool;

pub use client::{McpCallResult, McpClient, RemoteTool};
pub use tool::{validate_tool_arguments, McpTool};
