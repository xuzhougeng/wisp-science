//! `codex` — delegate a bounded subtask to the Codex CLI agent.
//!
//! Wisp's own agent stays in charge of the conversation; this tool hands one
//! self-contained task to `codex exec` running in the project directory.
//! Codex brings its own shell/file tools; Wisp's skills, bundled bio MCP, and
//! custom MCP connections reach it through the `wisp_bridge` stdio MCP server
//! configured in the per-project CODEX_HOME (see `codex_runtime`).

use crate::codex_runtime::{self, McpBridgeLaunch};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use wisp_llm::ToolSchema;
use wisp_tools::{Tool, ToolEnv, ToolEvent, ToolResult};

const DEFAULT_TIMEOUT_SECS: u64 = 600;
const MAX_TIMEOUT_SECS: u64 = 3600;
const MAX_RESULT_BYTES: usize = 16 * 1024;

pub struct CodexTool {
    app_data: PathBuf,
    project_id: String,
    frame_id: String,
}

impl CodexTool {
    pub fn new(app_data: PathBuf, project_id: String, frame_id: String) -> Self {
        Self {
            app_data,
            project_id,
            frame_id,
        }
    }

    fn bridge_launch(&self, project_root: &std::path::Path) -> Result<McpBridgeLaunch, String> {
        let exe = std::env::current_exe()
            .map_err(|e| format!("Cannot locate Wisp executable for MCP bridge: {e}"))?
            .display()
            .to_string();
        Ok(McpBridgeLaunch {
            command: exe,
            args: vec![
                "--wisp-mcp-bridge".to_string(),
                "--app-data".to_string(),
                self.app_data.display().to_string(),
                "--project-root".to_string(),
                project_root.display().to_string(),
                "--resource-root".to_string(),
                wisp_paths::resource_root().display().to_string(),
                "--project-id".to_string(),
                self.project_id.clone(),
                "--frame-id".to_string(),
                self.frame_id.clone(),
            ],
        })
    }
}

/// Whether the Codex CLI is installed — checked once per app run so the
/// model never sees a phantom tool it can't actually use.
pub async fn codex_cli_available() -> bool {
    static AVAILABLE: tokio::sync::OnceCell<bool> = tokio::sync::OnceCell::const_new();
    *AVAILABLE
        .get_or_init(|| async {
            let mut cmd = Command::new("codex");
            cmd.arg("--version")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            wisp_tools::process::hide_console_async(&mut cmd);
            let Ok(mut child) = cmd.spawn() else {
                return false;
            };
            match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
                Ok(Ok(status)) => status.success(),
                _ => {
                    let _ = child.start_kill();
                    false
                }
            }
        })
        .await
}

/// Resolves once the env's cancel flag is set (Stop button), polled at 100ms.
async fn cancel_watch(env: &dyn ToolEnv) {
    while !env.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Pull the fields we surface out of one `codex exec --json` event line.
/// Returns (agent_message_text, progress_note, error).
fn parse_codex_event(line: &str) -> (Option<String>, Option<String>, Option<String>) {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return (None, None, None);
    };
    let typ = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if typ == "error" || typ == "turn.failed" {
        let msg = v
            .get("message")
            .or_else(|| v.get("error"))
            .map(|m| {
                m.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| m.to_string())
            })
            .unwrap_or_else(|| "codex turn failed".into());
        return (None, None, Some(msg));
    }
    let item = v.get("item").unwrap_or(&v);
    let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let text = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| item.get(*k).and_then(|t| t.as_str()))
            .map(str::to_string)
    };
    match item_type {
        "agent_message" | "message" => (text(&["text", "content"]), None, None),
        "command_execution" => (
            None,
            text(&["command", "cmd"]).map(|c| format!("$ {c}")),
            None,
        ),
        "mcp_tool_call" | "tool_call" => (
            None,
            text(&["tool", "name"]).map(|t| format!("[tool] {t}")),
            None,
        ),
        _ => (None, None, None),
    }
}

fn truncate_middle(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let half = max / 2;
    let head_end = (0..=half)
        .rev()
        .find(|i| s.is_char_boundary(*i))
        .unwrap_or(0);
    let tail_start = (s.len() - half..s.len())
        .find(|i| s.is_char_boundary(*i))
        .unwrap_or(s.len());
    format!(
        "{}\n... [{} bytes truncated] ...\n{}",
        &s[..head_end],
        s.len() - head_end - (s.len() - tail_start),
        &s[tail_start..]
    )
}

#[async_trait]
impl Tool for CodexTool {
    fn name(&self) -> &str {
        "codex"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "codex",
            "Delegate one self-contained coding or analysis subtask to the Codex CLI agent, \
             which works autonomously in the project directory with its own shell and file \
             tools plus Wisp's skills and MCP tools (via the wisp_bridge MCP server). \
             Codex cannot see this conversation, so the task must carry all needed context. \
             Prefer it for large multi-file coding work; use Wisp's own tools for quick edits.",
            json!({
                "type": "object",
                "properties": {
                    "task": { "type": "string", "description": "Complete, self-contained task description including relevant paths and acceptance criteria" },
                    "timeout_secs": { "type": "integer", "description": "Max seconds to let codex run (default 600, max 3600)" }
                },
                "required": ["task"]
            }),
        )
    }

    fn preview(&self, args: &Value) -> String {
        args.get("task")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .chars()
            .take(150)
            .collect()
    }

    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        let Some(task) = args
            .get("task")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        else {
            return ToolResult::fail("codex error: missing required argument 'task'");
        };
        let timeout = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .clamp(30, MAX_TIMEOUT_SECS);

        let project_root = env.project_root().to_path_buf();
        let bridge = match self.bridge_launch(&project_root) {
            Ok(b) => b,
            Err(e) => return ToolResult::fail(format!("codex error: {e}")),
        };
        // Runtime prep copies ~/.codex — keep it off the async runtime.
        let runtime = {
            let root = project_root.clone();
            match tokio::task::spawn_blocking(move || {
                codex_runtime::prepare_codex_runtime(&root, &bridge)
            })
            .await
            {
                Ok(Ok(rt)) => rt,
                Ok(Err(e)) => return ToolResult::fail(format!("codex error: {e}")),
                Err(e) => return ToolResult::fail(format!("codex error: {e}")),
            }
        };

        // Non-interactive one-shot: task on stdin, JSONL events on stdout.
        // workspace-write sandbox: codex may edit the project but not escape it;
        // approval_policy=never because there is no interactive approver here.
        let mut command = Command::new("codex");
        command
            .arg("exec")
            .arg("--json")
            .arg("--cd")
            .arg(&project_root)
            .arg("--skip-git-repo-check")
            .arg("--sandbox")
            .arg("workspace-write")
            .arg("-c")
            .arg("approval_policy=\"never\"")
            .arg("-")
            .current_dir(&project_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in &runtime.env {
            command.env(k, v);
        }
        wisp_tools::process::hide_console_async(&mut command);

        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::fail(format!(
                    "codex error: failed to spawn `codex` (is Codex CLI installed and on PATH?): {e}"
                ));
            }
        };

        if let Some(mut stdin) = child.stdin.take() {
            if let Err(e) = stdin.write_all(task.as_bytes()).await {
                let _ = child.kill().await;
                return ToolResult::fail(format!("codex error: failed to send task: {e}"));
            }
            drop(stdin);
        }

        let mut stdout_reader = child.stdout.take().map(|s| BufReader::new(s).lines());
        let mut stderr_reader = child.stderr.take().map(|s| BufReader::new(s).lines());
        let mut stdout_done = stdout_reader.is_none();
        let mut stderr_done = stderr_reader.is_none();
        let mut messages: Vec<String> = vec![];
        let mut stderr_tail: Vec<String> = vec![];
        let mut error: Option<String> = None;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
        while !(stdout_done && stderr_done) {
            tokio::select! {
                _ = cancel_watch(env) => {
                    let _ = child.kill().await;
                    return ToolResult::fail("interrupted by user");
                }
                _ = tokio::time::sleep_until(deadline) => {
                    let _ = child.kill().await;
                    return ToolResult::fail(format!("codex timed out after {timeout}s"));
                }
                res = async { stdout_reader.as_mut().unwrap().next_line().await }, if !stdout_done => match res {
                    Ok(Some(line)) => {
                        let (msg, note, err) = parse_codex_event(&line);
                        if let Some(msg) = msg {
                            env.emit(ToolEvent::Stdout { chunk: format!("{msg}\n") }).await;
                            messages.push(msg);
                        }
                        if let Some(note) = note {
                            env.emit(ToolEvent::Stdout { chunk: format!("{note}\n") }).await;
                        }
                        if let Some(err) = err {
                            error = Some(err);
                        }
                    }
                    _ => stdout_done = true,
                },
                res = async { stderr_reader.as_mut().unwrap().next_line().await }, if !stderr_done => match res {
                    Ok(Some(line)) => {
                        stderr_tail.push(line);
                        if stderr_tail.len() > 50 {
                            stderr_tail.remove(0);
                        }
                    }
                    _ => stderr_done = true,
                },
            }
        }

        let status = tokio::select! {
            res = child.wait() => match res {
                Ok(s) => s,
                Err(e) => return ToolResult::fail(format!("codex error: {e}")),
            },
            _ = tokio::time::sleep_until(deadline) => {
                let _ = child.kill().await;
                return ToolResult::fail(format!("codex timed out after {timeout}s"));
            }
            _ = cancel_watch(env) => {
                let _ = child.kill().await;
                return ToolResult::fail("interrupted by user");
            }
        };

        if let Some(err) = error {
            return ToolResult::fail(truncate_middle(
                &format!("codex failed: {err}"),
                MAX_RESULT_BYTES,
            ));
        }
        if !status.success() {
            let detail = if stderr_tail.is_empty() {
                messages.join("\n\n")
            } else {
                stderr_tail.join("\n")
            };
            return ToolResult::fail(truncate_middle(
                &format!("codex exit {}: {detail}", status.code().unwrap_or(-1)),
                MAX_RESULT_BYTES,
            ));
        }
        let body = match messages.last() {
            Some(last) if messages.len() == 1 => last.clone(),
            Some(_) => messages.join("\n\n"),
            None => "(codex produced no final message)".into(),
        };
        let mut out = truncate_middle(&body, MAX_RESULT_BYTES);
        if !runtime.diagnostics.is_empty() {
            out.push_str("\n\n[runtime notes]\n");
            out.push_str(&runtime.diagnostics.join("\n"));
        }
        ToolResult::ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_agent_message_command_and_error_events() {
        let (msg, _, _) = parse_codex_event(
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"done"}}"#,
        );
        assert_eq!(msg.as_deref(), Some("done"));
        let (_, note, _) = parse_codex_event(
            r#"{"type":"item.started","item":{"type":"command_execution","command":"ls -la"}}"#,
        );
        assert_eq!(note.as_deref(), Some("$ ls -la"));
        let (_, _, err) = parse_codex_event(r#"{"type":"turn.failed","error":"boom"}"#);
        assert_eq!(err.as_deref(), Some("boom"));
        let (msg, note, err) = parse_codex_event("not json");
        assert!(msg.is_none() && note.is_none() && err.is_none());
    }

    #[test]
    fn truncates_long_results_on_char_boundaries() {
        let long = "中文内容".repeat(4096);
        let out = truncate_middle(&long, MAX_RESULT_BYTES);
        assert!(out.len() < long.len());
        assert!(out.contains("truncated"));
        // must not panic on boundaries; round-trip check
        assert!(out.starts_with('中'));
    }
}
