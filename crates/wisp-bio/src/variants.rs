//! Native `variants` domain. Implement `catalog` and `call` from operator APIs.
//! Python reference: mcp-servers/bio-tools/lib/mcp_variants/
use super::NativeBio;
use anyhow::{bail, Result};
use serde_json::Value;
use wisp_llm::ToolSchema;

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    Vec::new()
}

pub async fn call(_bio: &NativeBio, name: &str, _args: &Value) -> Result<Value> {
    bail!("unknown native biological tool: {name}")
}
