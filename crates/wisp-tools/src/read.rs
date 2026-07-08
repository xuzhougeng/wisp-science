//! `read` — read a text file with line numbers (offset/limit).

use crate::env::{ToolEnv, ToolResult};
use crate::tool::{arg_int_opt, arg_str, Tool};
use async_trait::async_trait;
use serde_json::json;
use std::path::Path;
use wisp_llm::ToolSchema;

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "read",
            "Read a file from the local filesystem. Image files are analyzed with the configured vision model.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to read (text, or image: png/jpg/jpeg/gif/webp)" },
                    "offset": { "type": "integer", "description": "Line number to start reading from (0-indexed, default 0)" },
                    "limit": { "type": "integer", "description": "Maximum number of lines to read (default: all lines)" }
                },
                "required": ["path"]
            }),
        )
    }
    fn preview(&self, args: &serde_json::Value) -> String {
        arg_str(args, "path").unwrap_or_default()
    }
    async fn run(&self, args: &serde_json::Value, _env: &dyn ToolEnv) -> ToolResult {
        let path = match arg_str(args, "path") {
            Ok(p) => p,
            Err(e) => return ToolResult::fail(e),
        };
        let ext = Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        if IMAGE_EXTS.contains(&ext.as_str())
            && arg_int_opt(args, "offset").is_none()
            && arg_int_opt(args, "limit").is_none()
        {
            return crate::image::view_image(&path);
        }
        // Cap checked before the read: a multi-GB log slurped whole can hang
        // or OOM the process.
        const MAX_READ_BYTES: u64 = 50 * 1024 * 1024;
        if let Ok(m) = std::fs::metadata(&path) {
            if m.len() > MAX_READ_BYTES {
                return ToolResult::fail(format!(
                    "read {path} error: file is {} bytes (limit {MAX_READ_BYTES}); use shell tools like head/tail/rg to sample it",
                    m.len()
                ));
            }
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => return ToolResult::fail(format!("read {path} error: {e}")),
        };
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = text.lines().collect();
        let offset = arg_int_opt(args, "offset").unwrap_or(0).max(0) as usize;
        let limit = arg_int_opt(args, "limit")
            .map(|l| l.max(0) as usize)
            .unwrap_or(lines.len());
        let selected = lines.iter().skip(offset).take(limit);
        let mut out = String::new();
        for (i, line) in selected.enumerate() {
            out.push_str(&format!("{:>4}| {}\n", offset + i + 1, line));
        }
        if out.is_empty() {
            out = "(empty file)".into();
        }
        ToolResult::ok(out)
    }
}
