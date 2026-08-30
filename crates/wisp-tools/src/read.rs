//! `read` — read a text file with line numbers (offset/limit).

use crate::env::{ToolEnv, ToolResult};
use crate::tool::{arg_int_opt, arg_str, Tool};
use async_trait::async_trait;
use serde_json::json;
use std::io::Read;
use wisp_llm::ToolSchema;

const MAX_READ_BYTES: u64 = 50 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

const BINARY_EXTS: &[&str] = &[
    "pdf", "doc", "docx", "docm", "odt", "rtf", "epub", "ppt", "pps", "pot", "pptx", "pptm",
    "ppsx", "ppsm", "xls", "xlsx", "xlsm", "xlsb", "ods", "odp", "zip", "gz", "7z", "tar", "exe",
    "dll", "so", "dylib", "wasm", "sqlite", "db",
];

/// Convert rich documents before the ordinary text/binary split. CSV stays raw:
/// scientific workflows generally need its exact delimiter and quoting semantics.
pub fn document_markdown(path: &std::path::Path, bytes: &[u8]) -> Option<Result<String, String>> {
    let format = anydoc::Format::from_path(path)?;
    if format == anydoc::Format::Csv {
        return None;
    }
    Some(anydoc::to_markdown_bytes(bytes, format).map_err(|error| error.to_string()))
}

fn looks_binary(path: &std::path::Path, bytes: &[u8]) -> bool {
    let by_ext = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| BINARY_EXTS.contains(&e.to_ascii_lowercase().as_str()));
    by_ext || bytes.starts_with(b"%PDF-") || bytes[..bytes.len().min(8192)].contains(&0)
}

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
            "Read a local text or document file. Office, OpenDocument, RTF, EPUB, and text-based PDF files are converted locally to Markdown. Sources are limited to 50 MiB and returned text to 1 MiB; images use the configured vision model.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to a text, document, or image file; relative paths resolve from the active project root" },
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
        if crate::image::is_supported_image(&path)
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
        let converted = document_markdown(&path, &bytes).and_then(Result::ok);
        if converted.is_none() && looks_binary(&path, &bytes) {
            return ToolResult::fail(format!(
                "read {requested_path} error: document has no locally extractable text; use OCR or a vision-capable model for scanned pages"
            ));
        }
        let text = converted.unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
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
    async fn rejects_binary_pdf() {
        let tmp = std::env::temp_dir().join(format!("wisp_read_pdf_{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();
        let pdf = tmp.join("doc.pdf");
        std::fs::write(&pdf, b"%PDF-1.7\n\x00\x00binary garbage").unwrap();

        let result = ReadTool
            .run(
                &json!({ "path": pdf.to_string_lossy() }),
                &TestEnv(tmp.clone()),
            )
            .await;
        assert!(!result.success);
        assert!(result.content.contains("no locally extractable text"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[tokio::test]
    async fn rich_text_documents_are_read_as_markdown() {
        let tmp = std::env::temp_dir().join(format!("wisp_read_rtf_{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();
        let rtf = tmp.join("protocol.rtf");
        std::fs::write(
            &rtf,
            br#"{\rtf1\ansi{\fonttbl{\f0 Arial;}}\b Experimental protocol\b0\par Centrifuge at 12000 g.}"#,
        )
        .unwrap();

        let result = ReadTool
            .run(
                &json!({ "path": rtf.to_string_lossy() }),
                &TestEnv(tmp.clone()),
            )
            .await;

        assert!(result.success, "{}", result.content);
        assert!(result.content.contains("**Experimental protocol**"));
        assert!(result.content.contains("Centrifuge at 12000 g."));
        assert!(!result.content.contains("\\rtf1"));
        std::fs::remove_dir_all(tmp).ok();
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

    #[tokio::test]
    async fn ordinary_reads_resolve_relative_paths_from_the_project_root() {
        let container = std::env::temp_dir().join(format!(
            "wisp_read_relative_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = container.join("project");
        let relative = PathBuf::from("analysis/scripts/example.R");
        std::fs::create_dir_all(root.join(relative.parent().unwrap())).unwrap();
        std::fs::write(root.join(&relative), "project relative").unwrap();
        let outside = container.join("outside.txt");
        std::fs::write(&outside, "absolute outside").unwrap();
        let env = TestEnv(root);

        let relative_result = ReadTool.run(&json!({"path": relative}), &env).await;
        assert!(relative_result.success, "{}", relative_result.content);
        assert!(relative_result.content.contains("project relative"));

        let absolute_result = ReadTool.run(&json!({"path": outside}), &env).await;
        assert!(absolute_result.success, "{}", absolute_result.content);
        assert!(absolute_result.content.contains("absolute outside"));

        std::fs::remove_dir_all(container).ok();
    }

    #[tokio::test]
    async fn logical_history_references_resolve_inside_the_working_project() {
        let root = std::env::temp_dir().join(format!(
            "wisp_read_history_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let history = root.join(".wisp/history");
        std::fs::create_dir_all(&history).unwrap();
        std::fs::write(history.join("archive-1.json"), "archived context").unwrap();

        let result = ReadTool
            .run(
                &json!({ "path": "wisp-history:archive-1" }),
                &TestEnv(root.clone()),
            )
            .await;
        assert!(result.success, "{}", result.content);
        assert!(result.content.contains("archived context"));
        assert!(
            !ReadTool
                .run(
                    &json!({ "path": "wisp-history:../outside" }),
                    &TestEnv(root.clone()),
                )
                .await
                .success
        );
        std::fs::remove_dir_all(root).ok();
    }
}
