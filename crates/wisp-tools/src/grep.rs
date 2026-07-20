//! `grep` — recursive regex content search.

use crate::env::{ToolEnv, ToolResult};
use crate::tool::{arg_str, arg_str_opt, Tool};
use async_trait::async_trait;
use regex::Regex;
use serde_json::json;
use std::io::Read;
use wisp_llm::ToolSchema;

const MAX_RESULTS: usize = 500;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const TRUNCATED: &str = "... results truncated";
const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

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
        let requested_base = arg_str_opt(args, "path")
            .unwrap_or_else(|| env.project_root().to_string_lossy().to_string());
        let base = match env.resolve_read_path(&requested_base, true) {
            Ok(path) => path,
            Err(error) => return ToolResult::fail(format!("grep error: {error}")),
        };
        let mut hits: Vec<String> = vec![];
        let mut hit_bytes = 0;
        let mut truncated = false;
        'walk: for entry in walkdir::WalkDir::new(&base)
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
            // ponytail: flat 10MB skip like code-search tools; no override knob
            // until someone actually greps huge files.
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
                continue;
            }
            let path = entry.path();
            let mut bytes = Vec::with_capacity(metadata.len() as usize);
            let Ok(_) = std::fs::File::open(path)
                .and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
            else {
                continue;
            };
            if bytes.len() as u64 > MAX_FILE_BYTES {
                truncated = true;
                break;
            }
            let Ok(text) = String::from_utf8(bytes) else {
                continue;
            };
            for (i, line) in text.lines().enumerate() {
                if re.is_match(line) {
                    let prefix = format!("{}:{}:", path.display(), i + 1);
                    if hits.len() >= MAX_RESULTS
                        || hit_bytes + prefix.len() + line.len() + 1
                            > MAX_OUTPUT_BYTES - TRUNCATED.len() - 1
                    {
                        truncated = true;
                        break 'walk;
                    }
                    hit_bytes += prefix.len() + line.len() + 1;
                    hits.push(format!("{prefix}{line}"));
                }
            }
        }
        let mut out = if hits.is_empty() {
            "none".to_string()
        } else {
            hits.join("\n")
        };
        if truncated {
            if out != "none" {
                out.push('\n');
            } else {
                out.clear();
            }
            out.push_str(TRUNCATED);
        }
        ToolResult::ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::ToolEvent;
    use std::path::{Path, PathBuf};

    struct TestEnv(PathBuf);

    #[async_trait::async_trait]
    impl ToolEnv for TestEnv {
        fn project_root(&self) -> &Path {
            &self.0
        }
        async fn confirm(&self, _message: &str) -> bool {
            true
        }
        async fn emit(&self, _event: ToolEvent) {}
    }

    #[tokio::test]
    async fn caps_a_matching_line_by_total_output_bytes() {
        let tmp = std::env::temp_dir().join(format!("wisp_grep_cap_{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("long.txt"), "x".repeat(MAX_OUTPUT_BYTES * 2)).unwrap();

        let result = GrepTool
            .run(&json!({ "pat": "x" }), &TestEnv(tmp.clone()))
            .await;
        assert!(result.success);
        assert!(result.content.contains(TRUNCATED));
        assert!(result.content.len() <= MAX_OUTPUT_BYTES);
        std::fs::remove_dir_all(&tmp).ok();
    }
}
