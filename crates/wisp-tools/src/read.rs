//! `read` — read a text file with line numbers (offset/limit).

use crate::env::{ToolEnv, ToolResult};
use crate::tool::{arg_int_opt, arg_str, Tool};
use async_trait::async_trait;
use serde_json::json;
use std::io::Read;
use wisp_llm::ToolSchema;

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];
const MAX_READ_BYTES: u64 = 50 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

fn render_lines(text: &str, offset: usize, limit: usize) -> String {
    const TRUNCATED: &str = "... output truncated at 1048576 bytes\n";
    let mut out = String::new();
    for (i, line) in text.lines().skip(offset).take(limit).enumerate() {
        let prefix = format!("{:>4}| ", offset + i + 1);
        if out.len() + prefix.len() + line.len() + 1 > MAX_OUTPUT_BYTES - TRUNCATED.len() {
            out.push_str(TRUNCATED);
            break;
        }
        out.push_str(&prefix);
        out.push_str(line);
        out.push('\n');
    }
    if out.is_empty() {
        "(empty file)".into()
    } else {
        out
    }
}

pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "read",
            "Read a local file. Text sources are limited to 50 MiB and returned text to 1 MiB; images use the configured vision model.",
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
    async fn run(&self, args: &serde_json::Value, env: &dyn ToolEnv) -> ToolResult {
        let requested_path = match arg_str(args, "path") {
            Ok(p) => p,
            Err(e) => return ToolResult::fail(e),
        };
        let path = match env.resolve_read_path(&requested_path, false) {
            Ok(path) => path,
            Err(error) => return ToolResult::fail(format!("read {requested_path} error: {error}")),
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        if IMAGE_EXTS.contains(&ext.as_str())
            && arg_int_opt(args, "offset").is_none()
            && arg_int_opt(args, "limit").is_none()
        {
            return crate::image::view_image(&path.to_string_lossy());
        }
        let metadata = match std::fs::metadata(&path) {
            Ok(m) if m.is_file() => m,
            Ok(_) => {
                return ToolResult::fail(format!("read {requested_path} error: not a regular file"))
            }
            Err(e) => return ToolResult::fail(format!("read {requested_path} error: {e}")),
        };
        if metadata.len() > MAX_READ_BYTES {
            return ToolResult::fail(format!(
                "read {requested_path} error: file is {} bytes (limit {MAX_READ_BYTES}); use shell tools like head/tail/rg to sample it",
                metadata.len()
            ));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        let read = std::fs::File::open(&path)
            .and_then(|file| file.take(MAX_READ_BYTES + 1).read_to_end(&mut bytes));
        match read {
            Ok(_) if bytes.len() as u64 <= MAX_READ_BYTES => {}
            Ok(_) => {
                return ToolResult::fail(format!(
                    "read {requested_path} error: file grew beyond {MAX_READ_BYTES} bytes while reading"
                ));
            }
            Err(e) => return ToolResult::fail(format!("read {requested_path} error: {e}")),
        }
        let text = String::from_utf8_lossy(&bytes);
        let offset = arg_int_opt(args, "offset").unwrap_or(0).max(0) as usize;
        let limit = arg_int_opt(args, "limit")
            .map(|l| l.max(0) as usize)
            .unwrap_or(usize::MAX);
        ToolResult::ok(render_lines(&text, offset, limit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::ToolEvent;
    use std::path::{Path, PathBuf};

    struct TestEnv(PathBuf);

    struct ScopedEnv(PathBuf);

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

    #[async_trait::async_trait]
    impl ToolEnv for ScopedEnv {
        fn project_root(&self) -> &Path {
            &self.0
        }
        fn restrict_read_paths_to_project(&self) -> bool {
            true
        }
        async fn confirm(&self, _message: &str) -> bool {
            true
        }
        async fn emit(&self, _event: ToolEvent) {}
    }

    #[test]
    fn render_lines_caps_a_single_long_line_without_indexing_all_lines() {
        let text = "x".repeat(MAX_OUTPUT_BYTES * 2);
        let out = render_lines(&text, 0, usize::MAX);
        assert!(out.len() <= MAX_OUTPUT_BYTES);
        assert!(out.contains("output truncated"));
    }

    #[tokio::test]
    async fn rejects_non_regular_files() {
        let tmp = std::env::temp_dir().join(format!("wisp_read_type_{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();

        let result = ReadTool
            .run(
                &json!({ "path": tmp.to_string_lossy() }),
                &TestEnv(tmp.clone()),
            )
            .await;
        assert!(!result.success);
        assert!(result.content.contains("not a regular file"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[tokio::test]
    async fn delegated_reads_reject_relative_and_absolute_scope_escape() {
        let container = std::env::temp_dir().join(format!(
            "wisp_read_scope_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::remove_dir_all(&container).ok();
        let root = container.join("project");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("inside.txt"), "inside").unwrap();
        let outside = container.join("outside.txt");
        std::fs::write(&outside, "outside").unwrap();
        let env = ScopedEnv(root.clone());

        assert!(
            ReadTool
                .run(&json!({"path":"inside.txt"}), &env)
                .await
                .success
        );
        assert!(
            !ReadTool
                .run(&json!({"path":"../outside.txt"}), &env)
                .await
                .success
        );
        assert!(!ReadTool.run(&json!({"path":outside}), &env).await.success);
        std::fs::remove_dir_all(container).ok();
    }
}
