//! `shell` — execute a shell command. On Windows this runs via PowerShell
//! (`powershell -NoProfile -Command`); the safety layer flags destructive
//! patterns for explicit confirmation. Output is capped and, for directory
//! traversals, filtered.

use crate::env::{ToolEnv, ToolEvent, ToolResult};
use crate::tool::{arg_str, Tool};
use async_trait::async_trait;
use serde_json::json;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{ChildStderr, ChildStdout, Command};
use wisp_llm::ToolSchema;

const TIMEOUT_SECS: u64 = 60;
const MAX_LINES: usize = 1000;

/// Resolves once the env's cancel flag is set. Polls at 100ms — cheap, and
/// bounds Stop-button latency to ~100ms while a command is mid-run.
async fn cancel_watch(env: &dyn ToolEnv) {
    while !env.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Read one line from an optional stdout stream.
async fn next_stdout_line(
    reader: &mut Option<tokio::io::Lines<BufReader<ChildStdout>>>,
) -> std::io::Result<Option<String>> {
    match reader {
        Some(r) => r.next_line().await,
        None => std::future::pending().await,
    }
}

/// Read one line from an optional stderr stream.
async fn next_stderr_line(
    reader: &mut Option<tokio::io::Lines<BufReader<ChildStderr>>>,
) -> std::io::Result<Option<String>> {
    match reader {
        Some(r) => r.next_line().await,
        None => std::future::pending().await,
    }
}

pub struct ShellTool;

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "shell",
            "Execute a shell command via PowerShell on Windows (60s timeout) and return stdout/stderr. Reach for this only when no dedicated tool fits.",
            json!({
                "type": "object",
                "properties": {
                    "cmd": { "type": "string", "description": "The shell command to execute, e.g. 'Get-ChildItem' or 'git status'" }
                },
                "required": ["cmd"]
            }),
        )
    }
    fn preview(&self, args: &serde_json::Value) -> String {
        arg_str(args, "cmd")
            .unwrap_or_default()
            .chars()
            .take(150)
            .collect()
    }

    async fn run(&self, args: &serde_json::Value, env: &dyn ToolEnv) -> ToolResult {
        let cmd = match arg_str(args, "cmd") {
            Ok(c) => c,
            Err(e) => return ToolResult::fail(e),
        };
        if let Some(danger) = crate::safety::check_command_safety(&cmd) {
            let msg = format!("Dangerous command detected ({}): {}", danger.label(), cmd);
            if !env.confirm(&msg).await {
                return ToolResult::fail("error: User denied action");
            }
        }

        env.emit(ToolEvent::Call {
            name: "shell".into(),
            preview: cmd.chars().take(150).collect(),
        })
        .await;

        let mut command = if cfg!(target_os = "windows") {
            let mut c = Command::new("powershell");
            c.arg("-NoProfile")
                .arg("-NonInteractive")
                .arg("-Command")
                .arg(&cmd);
            c
        } else {
            let mut c = Command::new("sh");
            c.arg("-c").arg(&cmd);
            c
        };
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        crate::process::hide_console_async(&mut command);
        command.current_dir(env.project_root());

        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(e) => return ToolResult::fail(format!("shell error: failed to spawn: {e}")),
        };

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let mut stdout_reader = stdout.map(|s| BufReader::new(s).lines());
        let mut stderr_reader = stderr.map(|s| BufReader::new(s).lines());
        let mut stdout_done = stdout_reader.is_none();
        let mut stderr_done = stderr_reader.is_none();
        let mut out_lines: Vec<String> = vec![];

        // Drain stdout and stderr concurrently so a silent stdout hang (e.g. ssh
        // waiting on stderr for host-key confirmation) cannot block cancel_watch.
        while !(stdout_done && stderr_done) {
            tokio::select! {
                _ = cancel_watch(env) => {
                    let _ = child.kill().await;
                    return ToolResult::fail("interrupted by user");
                }
                res = next_stdout_line(&mut stdout_reader), if !stdout_done => match res {
                    Ok(Some(line)) => {
                        env.emit(ToolEvent::Stdout {
                            chunk: format!("{line}\n"),
                        })
                        .await;
                        out_lines.push(line);
                    }
                    Ok(None) => stdout_done = true,
                    Err(_) => stdout_done = true,
                },
                res = next_stderr_line(&mut stderr_reader), if !stderr_done => match res {
                    Ok(Some(line)) => out_lines.push(line),
                    Ok(None) => stderr_done = true,
                    Err(_) => stderr_done = true,
                },
            }
            if out_lines.len() > MAX_LINES + 50 {
                let _ = child.kill().await;
                break;
            }
        }

        // ponytail: race child exit against the timeout and the cancel flag;
        // kill on either. cancel_watch polls at 100ms — a silent hang with no
        // output is still bounded by TIMEOUT_SECS.
        let status = tokio::select! {
            res = child.wait() => match res {
                Ok(s) => s,
                Err(e) => return ToolResult::fail(format!("shell error: {e}")),
            },
            _ = tokio::time::sleep(Duration::from_secs(TIMEOUT_SECS)) => {
                let _ = child.kill().await;
                return ToolResult::fail(format!("exec {cmd} timed out after {TIMEOUT_SECS}s"));
            }
            _ = cancel_watch(env) => {
                let _ = child.kill().await;
                return ToolResult::fail("interrupted by user");
            }
        };

        let out_lines = if crate::safety::is_directory_heavy(&cmd) {
            crate::safety::filter_directory_output(&out_lines, MAX_LINES)
        } else if out_lines.len() > MAX_LINES {
            let n = out_lines.len() - MAX_LINES;
            out_lines.truncate(MAX_LINES);
            out_lines.push(String::new());
            out_lines.push(format!("... and {n} more lines"));
            out_lines
        } else {
            out_lines
        };

        let body = out_lines.join("\n");
        if !status.success() {
            return ToolResult::fail(format!("exit {}: {body}", status.code().unwrap_or(-1)));
        }
        ToolResult::ok(if body.trim().is_empty() {
            "(empty)".to_string()
        } else {
            body
        })
    }
}
