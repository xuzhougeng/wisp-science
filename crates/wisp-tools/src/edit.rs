//! `edit` — replace an exact string in a file, with a unified-diff preview
//! and a uniqueness guard (matching mangopi's `edit` semantics).

use crate::env::{ToolEnv, ToolEvent, ToolResult};
use crate::tool::{arg_bool_opt, arg_str, Tool};
use async_trait::async_trait;
use serde_json::json;
use similar::TextDiff;
use wisp_llm::ToolSchema;

pub struct EditTool;

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "edit",
            "Edit a file by replacing an exact string with a new string. The `old` string must be unique unless `all` is true.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to edit" },
                    "old": { "type": "string", "description": "Exact string to be replaced" },
                    "new": { "type": "string", "description": "String to replace it with" },
                    "all": { "type": "boolean", "description": "Replace all occurrences (default: false)" }
                },
                "required": ["path", "old", "new"]
            }),
        )
    }
    fn preview(&self, args: &serde_json::Value) -> String {
        arg_str(args, "path").unwrap_or_default()
    }

    async fn before(&self, args: &serde_json::Value, env: &dyn ToolEnv) {
        let (Ok(path), Ok(old), Ok(new)) = (
            arg_str(args, "path"),
            arg_str(args, "old"),
            arg_str(args, "new"),
        ) else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let preview_new = text.replacen(&old, &new, 1);
        let diff = TextDiff::from_lines(&text, &preview_new)
            .unified_diff()
            .context_radius(3)
            .header(&format!("a/{path}"), &format!("b/{path}"))
            .to_string();
        let _ = diff.lines().take(200).count(); // cap preview cost
        env.emit(ToolEvent::Diff { path, old, new }).await;
    }

    async fn run(&self, args: &serde_json::Value, env: &dyn ToolEnv) -> ToolResult {
        let path = match arg_str(args, "path") {
            Ok(p) => p,
            Err(e) => return ToolResult::fail(e),
        };
        let old = match arg_str(args, "old") {
            Ok(o) => o,
            Err(e) => return ToolResult::fail(e),
        };
        let new = match arg_str(args, "new") {
            Ok(n) => n,
            Err(e) => return ToolResult::fail(e),
        };
        let all = arg_bool_opt(args, "all").unwrap_or(false);

        let real = match crate::safety::validate_file_path(env.project_root(), &path) {
            Ok(p) => p,
            Err(e) => return ToolResult::fail(format!("edit {path} error: {e}")),
        };
        if env.is_write_path_protected(&real) {
            return ToolResult::fail(format!(
                "edit {path} error: registered lab dossiers must be changed through lab_transaction"
            ));
        }
        let text = match std::fs::read_to_string(&real) {
            Ok(t) => t,
            Err(e) => return ToolResult::fail(format!("edit {path} error: {e}")),
        };
        if !text.contains(&old) {
            return ToolResult::fail("edit error: old_string not found");
        }
        let count = text.matches(&old).count();
        if !all && count > 1 {
            return ToolResult::fail(format!(
                "edit error: old_string appears {count} times, must be unique (use all=true)"
            ));
        }
        let replaced = if all {
            text.replace(&old, &new)
        } else {
            text.replacen(&old, &new, 1)
        };
        if let Err(e) = std::fs::write(&real, &replaced) {
            return ToolResult::fail(format!("edit {path} error: {e}"));
        }
        ToolResult::ok(format!(
            "edit {path} ok ({count} replacement{})",
            if count == 1 { "" } else { "s" }
        ))
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
    async fn registered_dossier_edit_is_rejected_without_mutation() {
        let root = std::env::temp_dir().join(format!(
            "wisp_protected_edit_{}_{}",
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
        std::fs::write(&protected, "original").unwrap();
        let env = ProtectedEnv {
            root: root.clone(),
            protected: protected.clone(),
        };
        let result = EditTool.run(&serde_json::json!({"path":"resources/AB-000001.md","old":"original","new":"tamper"}), &env).await;
        assert!(!result.success);
        assert_eq!(std::fs::read_to_string(&protected).unwrap(), "original");
        let _ = std::fs::remove_dir_all(root);
    }
}
