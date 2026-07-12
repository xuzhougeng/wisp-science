//! `write` — overwrite a file, sandboxed to the project root.

use crate::env::{ToolEnv, ToolResult};
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
        if env.is_write_path_protected(&real) {
            return ToolResult::fail(format!(
                "write {path} error: registered lab dossiers must be changed through lab_transaction"
            ));
        }
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
        ToolResult::ok(format!("write {} bytes to {} ok", content.len(), path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::ToolEvent;
    use std::path::{Path, PathBuf};

    struct ProtectedEnv {
        root: PathBuf,
        protected: PathBuf,
    }
    #[async_trait::async_trait]
    impl ToolEnv for ProtectedEnv {
        fn project_root(&self) -> &Path {
            &self.root
        }
        fn is_write_path_protected(&self, path: &Path) -> bool {
            path == self.protected
        }
        async fn confirm(&self, _: &str) -> bool {
            true
        }
        async fn emit(&self, _: ToolEvent) {}
    }

    #[tokio::test]
    async fn registered_dossier_path_is_rejected() {
        let root = std::env::temp_dir().join(format!(
            "wisp_protected_write_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("resources")).unwrap();
        let protected = dunce::canonicalize(root.join("resources"))
            .unwrap()
            .join("AB-000001.md");
        let env = ProtectedEnv {
            root: root.clone(),
            protected: protected.clone(),
        };
        let result = WriteTool
            .run(
                &serde_json::json!({"path":"resources/AB-000001.md","content":"tamper"}),
                &env,
            )
            .await;
        assert!(!result.success);
        assert!(!protected.exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
