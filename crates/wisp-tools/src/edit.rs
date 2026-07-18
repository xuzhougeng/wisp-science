//! `edit` — replace an exact string in a file, with a unified-diff preview
//! and a uniqueness guard (matching mangopi's `edit` semantics).

use crate::env::{ToolEnv, ToolEvent, ToolResult};
use crate::tool::{arg_bool_opt, arg_str, Tool};
use async_trait::async_trait;
use serde_json::json;
use std::io::Read;
use wisp_llm::ToolSchema;

const MAX_EDIT_BYTES: u64 = 10 * 1024 * 1024;

fn replaced_len(text: &str, old: &str, new: &str, all: bool) -> Option<usize> {
    let count = if all {
        if old.is_empty() {
            text.chars().count().checked_add(1)?
        } else {
            text.matches(old).count()
        }
    } else {
        1
    };
    let removed = old.len().checked_mul(count)?;
    let added = new.len().checked_mul(count)?;
    text.len().checked_sub(removed)?.checked_add(added)
}

pub struct EditTool;

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "edit",
            "Edit a text file up to 10 MiB by replacing an exact string. Read the file immediately before editing so `old` matches its current contents. The result must remain within 10 MiB, and `old` must be unique unless `all` is true.",
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
        if std::fs::metadata(&path).is_ok_and(|m| !m.is_file() || m.len() > MAX_EDIT_BYTES) {
            return;
        }
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
        if old.len() as u64 > MAX_EDIT_BYTES || new.len() as u64 > MAX_EDIT_BYTES {
            return ToolResult::fail(format!(
                "edit {path} error: replacement text exceeds {MAX_EDIT_BYTES} byte limit"
            ));
        }
        let all = arg_bool_opt(args, "all").unwrap_or(false);

        let real = match crate::safety::validate_file_path(env.project_root(), &path) {
            Ok(p) => p,
            Err(e) => return ToolResult::fail(format!("edit {path} error: {e}")),
        };
        let metadata = match std::fs::metadata(&real) {
            Ok(m) if m.is_file() => m,
            Ok(_) => return ToolResult::fail(format!("edit {path} error: not a regular file")),
            Err(e) => return ToolResult::fail(format!("edit {path} error: {e}")),
        };
        if metadata.len() > MAX_EDIT_BYTES {
            return ToolResult::fail(format!(
                "edit {path} error: file is {} bytes (limit {MAX_EDIT_BYTES})",
                metadata.len()
            ));
        }
        let mut text = String::with_capacity(metadata.len() as usize);
        let read = std::fs::File::open(&real)
            .and_then(|file| file.take(MAX_EDIT_BYTES + 1).read_to_string(&mut text));
        match read {
            Ok(n) if n as u64 <= MAX_EDIT_BYTES => {}
            Ok(_) => {
                return ToolResult::fail(format!(
                    "edit {path} error: file grew beyond {MAX_EDIT_BYTES} bytes while reading"
                ));
            }
            Err(e) => return ToolResult::fail(format!("edit {path} error: {e}")),
        }
        if !text.contains(&old) {
            return ToolResult::fail(
                "edit error: old string not found; re-read the file because it may have changed",
            );
        }
        let count = text.matches(&old).count();
        if !all && count > 1 {
            return ToolResult::fail(format!(
                "edit error: old_string appears {count} times, must be unique (use all=true)"
            ));
        }
        let Some(result_len) = replaced_len(&text, &old, &new, all) else {
            return ToolResult::fail("edit error: replacement size overflow");
        };
        if result_len as u64 > MAX_EDIT_BYTES {
            return ToolResult::fail(format!(
                "edit {path} error: edited file would be {result_len} bytes (limit {MAX_EDIT_BYTES})"
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
        env.emit(ToolEvent::FileChanged { path: path.clone() })
            .await;
        ToolResult::ok(format!(
            "edit {path} ok ({count} replacement{})",
            if count == 1 { "" } else { "s" }
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

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
    async fn rejects_large_files_before_reading_them() {
        let tmp = std::env::temp_dir().join(format!("wisp_edit_cap_{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("large.txt");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_EDIT_BYTES + 1)
            .unwrap();

        let result = EditTool
            .run(
                &json!({ "path": "large.txt", "old": "a", "new": "b" }),
                &TestEnv(tmp.clone()),
            )
            .await;
        assert!(!result.success);
        assert!(result.content.contains("limit"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[tokio::test]
    async fn successful_registry_edit_emits_file_changed_after_the_diff() {
        let tmp = std::env::temp_dir().join(format!("wisp_edit_events_{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("script.R"), "plot(1:3)\n").unwrap();
        let env = RecordingEnv {
            root: tmp.clone(),
            events: Mutex::new(Vec::new()),
        };

        let result = crate::Registry::builtins()
            .run(
                "edit",
                &json!({
                    "path": "script.R",
                    "old": "plot(1:3)",
                    "new": "plot(c(1, 2, 3), c(3, 1, 2))"
                }),
                &env,
            )
            .await;

        assert!(result.success, "{}", result.content);
        assert_eq!(
            std::fs::read_to_string(tmp.join("script.R")).unwrap(),
            "plot(c(1, 2, 3), c(3, 1, 2))\n"
        );
        let events = env.events.lock().unwrap();
        let diff = events
            .iter()
            .position(|event| matches!(event, ToolEvent::Diff { .. }))
            .expect("diff event");
        let changed = events
            .iter()
            .position(
                |event| matches!(event, ToolEvent::FileChanged { path } if path == "script.R"),
            )
            .expect("post-write file-changed event");
        assert!(
            diff < changed,
            "refresh signal must follow the pre-write diff"
        );
        drop(events);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn replacement_size_is_checked_before_allocating() {
        assert_eq!(replaced_len("aaa", "a", "bb", true), Some(6));
        assert_eq!(replaced_len("abc", "", "x", true), Some(7));
        assert_eq!(replaced_len("aaa", "a", "bb", false), Some(4));
        assert!(
            replaced_len("aaa", "a", &"x".repeat(MAX_EDIT_BYTES as usize), true)
                .is_some_and(|len| len as u64 > MAX_EDIT_BYTES)
        );
    }
}
