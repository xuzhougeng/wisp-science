//! `edit` — replace an exact string in a file, with a unified-diff preview
//! and a uniqueness guard (matching mangopi's `edit` semantics).

use crate::env::{ToolEnv, ToolEvent, ToolResult};
use crate::tool::{arg_bool_opt, arg_str, Tool};
use async_trait::async_trait;
use serde_json::json;
use std::io::Read;
use wisp_llm::ToolSchema;

const MAX_EDIT_BYTES: u64 = 10 * 1024 * 1024;
/// Files larger than this skip the closest-match scan: the Levenshtein pass
/// over every line window gets too expensive to justify better error text.
const MAX_CLOSEST_SCAN_BYTES: usize = 1024 * 1024;
const CLOSEST_MATCH_MIN_SIMILARITY: f64 = 0.5;
const CLOSEST_SNIPPET_MAX_LINES: usize = 20;

fn char_levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr: Vec<usize> = vec![0; b.len() + 1];
    for (i, ca) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            curr[j + 1] = (prev[j] + usize::from(ca != *cb))
                .min(prev[j + 1] + 1)
                .min(curr[j] + 1);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// Best contiguous line window resembling `old`, as
/// `(1-based start line, end line, similarity, snippet)` when the similarity
/// reaches [`CLOSEST_MATCH_MIN_SIMILARITY`].
fn find_closest_match(text: &str, old: &str) -> Option<(usize, usize, f64, String)> {
    if text.len() > MAX_CLOSEST_SCAN_BYTES || old.is_empty() {
        return None;
    }
    let old_lines: Vec<&str> = old.lines().collect();
    let file_lines: Vec<&str> = text.lines().collect();
    let window = old_lines.len();
    if old_lines.is_empty() || window > file_lines.len() {
        return None;
    }
    let old_joined = old_lines.join("\n");
    let target_len = old_joined.chars().count();
    let mut best: Option<(usize, f64)> = None;
    for start in 0..=(file_lines.len() - window) {
        let candidate = file_lines[start..start + window].join("\n");
        let candidate_len = candidate.chars().count();
        // A window this far off in length cannot reach the similarity floor.
        if candidate_len.abs_diff(target_len) > target_len / 2 + 8 {
            continue;
        }
        let distance = char_levenshtein(&candidate, &old_joined);
        let similarity = 1.0 - distance as f64 / candidate_len.max(target_len).max(1) as f64;
        if best.map_or(true, |(_, s)| similarity > s) {
            best = Some((start, similarity));
        }
    }
    let (start, similarity) = best?;
    if similarity < CLOSEST_MATCH_MIN_SIMILARITY {
        return None;
    }
    let lines = &file_lines[start..start + window];
    let snippet = if lines.len() > CLOSEST_SNIPPET_MAX_LINES {
        let mut s = lines[..CLOSEST_SNIPPET_MAX_LINES].join("\n");
        s.push_str(&format!(
            "\n... ({} more lines)",
            lines.len() - CLOSEST_SNIPPET_MAX_LINES
        ));
        s
    } else {
        lines.join("\n")
    };
    Some((start + 1, start + window, similarity, snippet))
}

fn not_found_message(path: &str, text: &str, old: &str) -> String {
    match find_closest_match(text, old) {
        Some((start, end, similarity, snippet)) => format!(
            "edit error: old string not found in {path}. Closest match at lines {start}-{end} \
(similarity {}%):\n```\n{snippet}\n```\nThe file may have changed — re-read the exact region \
and retry with the current text.",
            (similarity * 100.0).round() as u32
        ),
        None => format!(
            "edit error: old string not found in {path} ({} lines); re-read the file because it may have changed",
            text.lines().count().max(1)
        ),
    }
}

/// 1-based line numbers of the first `limit` occurrences of `old` in `text`.
fn match_line_numbers(text: &str, old: &str, limit: usize) -> Vec<usize> {
    text.match_indices(old)
        .take(limit)
        .map(|(idx, _)| text[..idx].bytes().filter(|b| *b == b'\n').count() + 1)
        .collect()
}

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
        if env.is_cancelled() {
            return ToolResult::fail("interrupted by user");
        }
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
            return ToolResult::fail(not_found_message(&path, &text, &old));
        }
        let count = text.matches(&old).count();
        if !all && count > 1 {
            let lines = match_line_numbers(&text, &old, 5);
            return ToolResult::fail(format!(
                "edit error: old_string appears {count} times, must be unique (use all=true); \
matches at lines {}",
                lines
                    .iter()
                    .map(|l| l.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
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
        if let Err(e) = crate::safety::write_no_follow(&real, replaced.as_bytes()) {
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

    #[tokio::test]
    async fn crlf_multibyte_edit_preserves_line_endings_and_applies_once() {
        let tmp = std::env::temp_dir().join(format!("wisp_edit_crlf_{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();
        let original = "第一行：概要\r\n実験結果：陽性\r\n最終行：完了\r\n";
        std::fs::write(tmp.join("report.txt"), original.as_bytes()).unwrap();

        let result = EditTool
            .run(
                &json!({
                    "path": "report.txt",
                    "old": "実験結果：陽性",
                    "new": "実験結果：陰性"
                }),
                &TestEnv(tmp.clone()),
            )
            .await;

        assert!(result.success, "{}", result.content);
        assert!(
            result.content.contains("1 replacement"),
            "{}",
            result.content
        );
        let bytes = std::fs::read(tmp.join("report.txt")).unwrap();
        assert_eq!(
            bytes,
            "第一行：概要\r\n実験結果：陰性\r\n最終行：完了\r\n".as_bytes(),
            "CRLF endings and surrounding CJK text must survive the edit byte-for-byte"
        );
        assert_eq!(
            bytes.windows(2).filter(|w| w == b"\r\n").count(),
            3,
            "all three CRLF terminators preserved"
        );
        std::fs::remove_dir_all(&tmp).ok();
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
    async fn cancelled_edit_does_not_change_the_file() {
        let tmp = std::env::temp_dir().join(format!("wisp_edit_cancel_{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("keep.txt"), "original\n").unwrap();
        let env = CancelledEnv(tmp.clone());

        let result = EditTool
            .run(
                &json!({
                    "path": "keep.txt",
                    "old": "original",
                    "new": "changed"
                }),
                &env,
            )
            .await;

        assert!(!result.success, "{}", result.content);
        assert!(result.content.contains("interrupted by user"));
        assert_eq!(
            std::fs::read_to_string(tmp.join("keep.txt")).unwrap(),
            "original\n"
        );
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

    #[tokio::test]
    async fn not_found_error_reports_closest_near_miss_match() {
        let tmp = std::env::temp_dir().join(format!("wisp_edit_closest_{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();
        let content = "def compute_total(items):\n    total = 0\n    for item in items:\n        total += item.price\n    return total\n";
        std::fs::write(tmp.join("cart.py"), content).unwrap();

        // One character off (`cost` vs `price`) with identical indentation.
        let result = EditTool
            .run(
                &json!({
                    "path": "cart.py",
                    "old": "        total += item.cost",
                    "new": "        total += 0"
                }),
                &TestEnv(tmp.clone()),
            )
            .await;

        assert!(!result.success);
        assert!(
            result.content.contains("Closest match at lines 4-4"),
            "{}",
            result.content
        );
        assert!(
            result.content.contains("        total += item.price"),
            "snippet must keep the original indentation: {}",
            result.content
        );
        assert!(
            result
                .content
                .contains("re-read the exact region and retry with the current text"),
            "{}",
            result.content
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[tokio::test]
    async fn not_found_error_without_closest_match_keeps_re_read_hint() {
        let tmp = std::env::temp_dir().join(format!("wisp_edit_unrelated_{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("notes.txt"), "alpha\nbeta\ngamma\n").unwrap();

        let result = EditTool
            .run(
                &json!({
                    "path": "notes.txt",
                    "old": "zzz completely unrelated qqq",
                    "new": "x"
                }),
                &TestEnv(tmp.clone()),
            )
            .await;

        assert!(!result.success);
        assert!(
            !result.content.contains("Closest match"),
            "{}",
            result.content
        );
        assert!(
            result
                .content
                .contains("re-read the file because it may have changed"),
            "{}",
            result.content
        );
        assert!(result.content.contains("(3 lines)"), "{}", result.content);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[tokio::test]
    async fn ambiguous_old_string_error_lists_match_line_numbers() {
        let tmp = std::env::temp_dir().join(format!("wisp_edit_ambiguous_{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("dup.txt"),
            "dup here\nmiddle\ndup here\nx\ndup here\n",
        )
        .unwrap();

        let result = EditTool
            .run(
                &json!({ "path": "dup.txt", "old": "dup here", "new": "unique" }),
                &TestEnv(tmp.clone()),
            )
            .await;

        assert!(!result.success);
        assert!(
            result.content.contains("appears 3 times"),
            "{}",
            result.content
        );
        assert!(
            result.content.contains("matches at lines 1, 3, 5"),
            "{}",
            result.content
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn char_levenshtein_ratio_and_scan_guards() {
        assert_eq!(char_levenshtein("kitten", "sitting"), 3);
        assert_eq!(char_levenshtein("same", "same"), 0);
        // Unrelated short window must fall below the similarity floor.
        assert!(find_closest_match("alpha\nbeta\n", "zzz unrelated qqq").is_none());
        // Oversized files skip the scan entirely.
        let big = "x\n".repeat(MAX_CLOSEST_SCAN_BYTES / 2 + 2);
        assert!(find_closest_match(&big, "x").is_none());
    }
}
