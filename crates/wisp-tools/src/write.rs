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
            "Write content to a file, overwriting if it exists. The path must be inside the project root.",
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
        if let Err(e) = std::fs::write(&real, &content) {
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
}
