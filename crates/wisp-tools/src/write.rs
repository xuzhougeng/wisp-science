//! `write` — overwrite a file, sandboxed to the project root.

use crate::env::{ToolEnv, ToolEvent, ToolResult};
use crate::tool::{arg_str, Tool};
use async_trait::async_trait;
use serde_json::json;
use wisp_llm::ToolSchema;

pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "write",
            "Write content to a file, overwriting if it exists. The path must be inside the project root. `content` counts against the model's output limit, so an oversized call is cut off and discarded: revise an existing long document with edit, section by section, instead of rewriting it.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to write" },
                    "content": { "type": "string", "description": "Content to write to the file" }
                },
                "required": ["path", "content"]
            }),
        )
    }
    fn preview(&self, args: &serde_json::Value) -> String {
        arg_str(args, "path").unwrap_or_default()
    }
    async fn run(&self, args: &serde_json::Value, env: &dyn ToolEnv) -> ToolResult {
        if env.is_cancelled() {
            return ToolResult::fail("interrupted by user");
        }
        let path = match arg_str(args, "path") {
            Ok(p) => p,
            Err(e) => return ToolResult::fail(e),
        };
        let content = match arg_str(args, "content") {
            Ok(c) => c,
            Err(e) => return ToolResult::fail(e),
        };
        let real = match crate::safety::validate_file_path(env.project_root(), &path) {
            Ok(p) => p,
            Err(e) => return ToolResult::fail(format!("write {path} error: {e}")),
        };
        if let Some(parent) = real.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return ToolResult::fail(format!(
                    "write {path} error: cannot create parent dir: {e}"
                ));
            }
        }
        if let Err(e) = crate::safety::write_no_follow(&real, content.as_bytes()) {
            return ToolResult::fail(format!("write {path} error: {e}"));
        }
        env.emit(ToolEvent::FileChanged { path: path.clone() })
            .await;
        ToolResult::ok(format!("write {} bytes to {} ok", content.len(), path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    struct RecordingEnv {
        root: PathBuf,
        events: Mutex<Vec<ToolEvent>>,
    }

    #[async_trait::async_trait]
    impl ToolEnv for RecordingEnv {
        fn project_root(&self) -> &Path {
            &self.root
        }
        async fn confirm(&self, _message: &str) -> bool {
            true
        }
        async fn emit(&self, event: ToolEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[tokio::test]
    async fn successful_write_emits_file_changed() {
        let tmp = std::env::temp_dir().join(format!("wisp_write_events_{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();
        let env = RecordingEnv {
            root: tmp.clone(),
            events: Mutex::new(Vec::new()),
        };

        let result = WriteTool
            .run(&json!({ "path": "new.R", "content": "x <- 1\n" }), &env)
            .await;

        assert!(result.success, "{}", result.content);
        assert!(env
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, ToolEvent::FileChanged { path } if path == "new.R")));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_through_a_dangling_outbound_symlink_is_blocked() {
        let base = std::env::temp_dir().join(format!("wisp_write_dangling_{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        let root = base.join("project");
        let outside = base.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        // Project-local link to a nonexistent file outside the root: following
        // it would create /outside/escape.txt.
        std::os::unix::fs::symlink(outside.join("escape.txt"), root.join("sneaky.txt")).unwrap();
        let env = RecordingEnv {
            root: root.clone(),
            events: Mutex::new(Vec::new()),
        };

        let result = WriteTool
            .run(&json!({ "path": "sneaky.txt", "content": "pwned" }), &env)
            .await;

        assert!(!result.success, "{}", result.content);
        assert!(
            !outside.join("escape.txt").exists(),
            "the escape target must not be created"
        );
        assert!(env.events.lock().unwrap().is_empty());
        std::fs::remove_dir_all(&base).ok();
    }

    struct CancelledEnv(PathBuf);

    #[async_trait::async_trait]
    impl ToolEnv for CancelledEnv {
        fn project_root(&self) -> &Path {
            &self.0
        }
        async fn confirm(&self, _message: &str) -> bool {
            true
        }
        async fn emit(&self, _event: ToolEvent) {}
        fn is_cancelled(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn cancelled_write_does_not_touch_the_file() {
        let tmp = std::env::temp_dir().join(format!("wisp_write_cancel_{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();
        let env = CancelledEnv(tmp.clone());

        let result = WriteTool
            .run(&json!({ "path": "skip.txt", "content": "nope" }), &env)
            .await;

        assert!(!result.success, "{}", result.content);
        assert!(result.content.contains("interrupted by user"));
        assert!(!tmp.join("skip.txt").exists());
        std::fs::remove_dir_all(&tmp).ok();
    }
}
