//! `search` — glob file search, sorted by mtime (newest first).

use crate::env::{ToolEnv, ToolResult};
use crate::tool::{arg_str, arg_str_opt, Tool};
use async_trait::async_trait;
use serde_json::json;
use std::{cmp::Reverse, collections::BinaryHeap};
use wisp_llm::ToolSchema;

const MAX_RESULTS: usize = 500;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const TRUNCATED: &str = "... results truncated";

fn retain_newest(
    hits: &mut BinaryHeap<Reverse<(std::time::SystemTime, String)>>,
    hit: (std::time::SystemTime, String),
) {
    hits.push(Reverse(hit));
    if hits.len() > MAX_RESULTS {
        hits.pop();
    }
}

pub struct SearchTool;

#[async_trait]
impl Tool for SearchTool {
    fn name(&self) -> &str {
        "search"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "search",
            "Search for files using a glob pattern (e.g. '**/*.rs'). Returns at most the newest 500 matches within a 1 MiB output limit.",
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
        if env.restrict_read_paths_to_project() {
            if let Err(error) = crate::safety::validate_relative_pattern(&pat) {
                return ToolResult::fail(format!("search error: {error}"));
            }
        }
        let requested_base = arg_str_opt(args, "path")
            .unwrap_or_else(|| env.project_root().to_string_lossy().to_string());
        let base = match env.resolve_read_path(&requested_base, true) {
            Ok(path) => path,
            Err(error) => return ToolResult::fail(format!("search error: {error}")),
        };
        let full = base.join(&pat).to_string_lossy().replace("\\\\", "\\");
        let mut hits = BinaryHeap::new();
        let mut match_count = 0usize;
        for entry in glob::glob(&full).ok().into_iter().flatten().flatten() {
            let path = entry.to_string_lossy().to_string();
            let mtime = std::fs::metadata(&entry)
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(std::time::UNIX_EPOCH);
            match_count = match_count.saturating_add(1);
            retain_newest(&mut hits, (mtime, path));
        }
        let mut hits = hits.into_iter().map(|Reverse(hit)| hit).collect::<Vec<_>>();
        hits.sort_by(|a, b| b.0.cmp(&a.0));
        let mut truncated = match_count > hits.len();
        let mut out = String::new();
        for (_, path) in hits {
            if out.len() + path.len() + 1 > MAX_OUTPUT_BYTES - TRUNCATED.len() - 1 {
                truncated = true;
                break;
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&path);
        }
        if out.is_empty() && !truncated {
            out.push_str("none");
        }
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
    async fn caps_results_before_collecting_the_whole_glob() {
        let tmp = std::env::temp_dir().join(format!("wisp_search_cap_{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();
        for i in 0..=MAX_RESULTS {
            std::fs::write(tmp.join(format!("{i:04}.txt")), b"").unwrap();
        }

        let result = SearchTool
            .run(&json!({ "pat": "*.txt" }), &TestEnv(tmp.clone()))
            .await;
        assert!(result.success);
        assert!(result.content.contains("results truncated"));
        assert_eq!(result.content.lines().count(), MAX_RESULTS + 1);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn bounded_selection_keeps_the_newest_hits() {
        let mut hits = BinaryHeap::new();
        for i in 0..=MAX_RESULTS {
            retain_newest(
                &mut hits,
                (
                    std::time::UNIX_EPOCH + std::time::Duration::from_secs(i as u64),
                    i.to_string(),
                ),
            );
        }

        assert_eq!(hits.len(), MAX_RESULTS);
        assert!(!hits.iter().any(|Reverse((_, path))| path == "0"));
        assert!(hits
            .iter()
            .any(|Reverse((_, path))| path == &MAX_RESULTS.to_string()));
    }
}
