//! `grep` — recursive regex content search.

use crate::env::{ToolEnv, ToolResult};
use crate::tool::{arg_str, arg_str_opt, Tool};
use async_trait::async_trait;
use regex::Regex;
use serde_json::json;
use wisp_llm::ToolSchema;

pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "grep",
            "Search file contents recursively using a regular expression pattern (Rust `regex` syntax).",
            json!({
                "type": "object",
                "properties": {
                    "pat": { "type": "string", "description": "Regular expression pattern to search for (Rust regex syntax)" },
                    "path": { "type": "string", "description": "Search directory to recurse (defaults to project root)" }
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
        let re = match Regex::new(&pat) {
            Ok(r) => r,
            Err(e) => return ToolResult::fail(format!("grep error: invalid regex: {e}")),
        };
        let base = arg_str_opt(args, "path")
            .unwrap_or_else(|| env.project_root().to_string_lossy().to_string());
        let mut hits: Vec<String> = vec![];
        for entry in walkdir::WalkDir::new(&base)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                !(e.depth() > 0 && crate::safety::FILTERED_DIRS.contains(&name.as_ref()))
            })
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            for (i, line) in text.lines().enumerate() {
                if re.is_match(line) {
                    hits.push(format!("{}:{}:{}", path.display(), i + 1, line));
                    if hits.len() >= 500 {
                        break;
                    }
                }
            }
            if hits.len() >= 500 {
                break;
            }
        }
        let out = if hits.is_empty() {
            "none".to_string()
        } else {
            hits.join("\n")
        };
        ToolResult::ok(out)
    }
}
