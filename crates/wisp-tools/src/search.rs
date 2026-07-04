//! `search` — glob file search, sorted by mtime (newest first).

use crate::env::{ToolEnv, ToolResult};
use crate::tool::{arg_str, arg_str_opt, Tool};
use async_trait::async_trait;
use serde_json::json;
use std::path::Path;
use wisp_llm::ToolSchema;

pub struct SearchTool;

#[async_trait]
impl Tool for SearchTool {
    fn name(&self) -> &str {
        "search"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "search",
            "Search for files using a glob pattern (e.g. '**/*.rs'). Results are sorted by modification time, newest first.",
            json!({
                "type": "object",
                "properties": {
                    "pat": { "type": "string", "description": "Glob pattern to match file paths (e.g. '**/*.py')" },
                    "path": { "type": "string", "description": "Directory to start search from (default: project root)" }
                },
                "required": ["pat"]
            }),
        )
    }
    fn preview(&self, args: &serde_json::Value) -> String {
        arg_str(args, "pat").unwrap_or_default()
    }
    async fn run(&self, args: &serde_json::Value, env: &dyn ToolEnv) -> ToolResult {
        let pat = match arg_str(args, "pat") {
            Ok(p) => p,
            Err(e) => return ToolResult::fail(e),
        };
        let base = arg_str_opt(args, "path")
            .unwrap_or_else(|| env.project_root().to_string_lossy().to_string());
        let full = Path::new(&base)
            .join(&pat)
            .to_string_lossy()
            .replace("\\\\", "\\");
        let mut hits: Vec<(std::time::SystemTime, String)> = vec![];
        for entry in glob::glob(&full).ok().into_iter().flatten().flatten() {
            let mtime = std::fs::metadata(&entry)
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(std::time::UNIX_EPOCH);
            hits.push((mtime, entry.to_string_lossy().to_string()));
        }
        hits.sort_by(|a, b| b.0.cmp(&a.0));
        let out = if hits.is_empty() {
            "none".to_string()
        } else {
            hits.into_iter()
                .map(|(_, p)| p)
                .collect::<Vec<_>>()
                .join("\n")
        };
        ToolResult::ok(out)
    }
}
