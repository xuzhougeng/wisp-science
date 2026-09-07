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
use tokio::io::AsyncReadExt;
use tokio::process::{ChildStderr, ChildStdout, Command};
use wisp_llm::ToolSchema;

const TIMEOUT_SECS: u64 = 60;
const MAX_LINES: usize = 1000;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

/// Resolves once the env's cancel flag is set. Polls at 100ms — cheap, and
/// bounds Stop-button latency to ~100ms while a command is mid-run.
async fn cancel_watch(env: &dyn ToolEnv) {
    while !env.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn read_stdout(reader: &mut Option<ChildStdout>, buf: &mut [u8]) -> std::io::Result<usize> {
    match reader {
        Some(r) => r.read(buf).await,
        None => std::future::pending().await,
    }
}

async fn read_stderr(reader: &mut Option<ChildStderr>, buf: &mut [u8]) -> std::io::Result<usize> {
    match reader {
        Some(r) => r.read(buf).await,
        None => std::future::pending().await,
    }
}

fn append_output(output: &mut Vec<u8>, chunk: &[u8], remaining: usize) -> (usize, bool) {
    let take = chunk.len().min(remaining);
    output.extend_from_slice(&chunk[..take]);
    (take, take < chunk.len())
}

fn decode_stdout_chunk(carry: &mut Vec<u8>, chunk: &[u8], flush: bool) -> String {
    carry.extend_from_slice(chunk);
    let valid = match std::str::from_utf8(carry) {
        Ok(_) => carry.len(),
        Err(error) if error.error_len().is_none() => error.valid_up_to(),
        Err(_) => carry.len(),
    };
    let mut text = String::from_utf8_lossy(&carry[..valid]).into_owned();
    carry.drain(..valid);
    if flush && !carry.is_empty() {
        text.push_str(&String::from_utf8_lossy(carry));
        carry.clear();
    }
    text
}

pub struct ShellTool;

fn shell_description() -> String {
    let shell = if cfg!(target_os = "windows") {
        "PowerShell"
    } else {
        "POSIX sh"
    };
    format!("Execute a shell command via {shell} (60s timeout, 1 MiB combined output limit) and return stdout/stderr. Reach for this only when no dedicated tool fits. Write commands for this OS; avoid cross-shell one-liners and use Python or pixi for package-heavy scientific work.")
}

async fn run_shell(args: &serde_json::Value, env: &dyn ToolEnv, timeout: Duration) -> ToolResult {
    let cmd = match arg_str(args, "cmd") {
        Ok(c) => c,
        Err(e) => return ToolResult::fail(e),
    };
    if let Err(error) = env.preflight_local_execution(&cmd).await {
        return ToolResult::fail(error).stop_batch();
    }
    if let Err(error) = env.preflight_shell(&cmd).await {
        return ToolResult::fail(error);
    }
    // In the "full" scope dangerous commands run without a prompt; otherwise
    // ("auto" and "ask") a dangerous command still asks.
    if !env.danger_auto_approve() {
        if let Some(danger) = crate::safety::check_command_safety(&cmd) {
            let msg = format!("Dangerous command detected ({}): {}", danger.label(), cmd);
            if !env.confirm(&msg).await {
                return ToolResult::fail("error: User denied action").stop_batch();
            }
        }
    }

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
    command.envs(crate::network::command_proxy_env());

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => return ToolResult::fail(format!("shell error: failed to spawn: {e}")),
    };
    let deadline = tokio::time::sleep(timeout);
    let cancelled = cancel_watch(env);
    tokio::pin!(deadline, cancelled);

    let mut stdout_reader = child.stdout.take();
    let mut stderr_reader = child.stderr.take();
    let mut stdout_done = stdout_reader.is_none();
    let mut stderr_done = stderr_reader.is_none();
    let mut stdout_buf = [0_u8; 8192];
    let mut stderr_buf = [0_u8; 8192];
    let mut stdout_output = Vec::new();
    let mut stderr_output = Vec::new();
    let mut stdout_carry = Vec::with_capacity(4);
    let mut output_limited = false;

    // One deadline covers both output draining and the final child wait.
    while !(stdout_done && stderr_done) {
        tokio::select! {
            _ = &mut deadline => {
                let _ = child.kill().await;
                let detail = format!(
                    "exec {cmd} timed out after {}s",
                    timeout.as_secs_f64()
                );
                env.note_shell_outcome(&cmd, false, &detail);
                return ToolResult::fail(detail);
            }
            _ = &mut cancelled => {
                let _ = child.kill().await;
                return ToolResult::fail("interrupted by user");
            }
            res = read_stdout(&mut stdout_reader, &mut stdout_buf), if !stdout_done => match res {
                Ok(0) => {
                    stdout_done = true;
                    let chunk = decode_stdout_chunk(&mut stdout_carry, &[], true);
                    if !chunk.is_empty() {
                        env.emit(ToolEvent::Stdout { chunk }).await;
                    }
                },
                Ok(n) => {
                    let remaining = MAX_OUTPUT_BYTES
                        .saturating_sub(stdout_output.len() + stderr_output.len());
                    let (kept, limited) =
                        append_output(&mut stdout_output, &stdout_buf[..n], remaining);
                    if kept > 0 {
                        let chunk = decode_stdout_chunk(
                            &mut stdout_carry,
                            &stdout_buf[..kept],
                            false,
                        );
                        if !chunk.is_empty() {
                            env.emit(ToolEvent::Stdout { chunk }).await;
                        }
                    }
                    output_limited |= limited;
                }
                Err(_) => stdout_done = true,
            },
            res = read_stderr(&mut stderr_reader, &mut stderr_buf), if !stderr_done => match res {
                Ok(0) => stderr_done = true,
                Ok(n) => {
                    let remaining = MAX_OUTPUT_BYTES
                        .saturating_sub(stdout_output.len() + stderr_output.len());
                    output_limited |=
                        append_output(&mut stderr_output, &stderr_buf[..n], remaining).1;
                },
                Err(_) => stderr_done = true,
            },
        }
        if output_limited {
            let _ = child.kill().await;
            break;
        }
    }

    let chunk = decode_stdout_chunk(&mut stdout_carry, &[], true);
    if !chunk.is_empty() {
        env.emit(ToolEvent::Stdout { chunk }).await;
    }

    let status = tokio::select! {
        res = child.wait() => match res {
            Ok(s) => s,
            Err(e) => {
                let detail = format!("shell error: {e}");
                env.note_shell_outcome(&cmd, false, &detail);
                return ToolResult::fail(detail);
            }
        },
        _ = &mut deadline => {
            let _ = child.kill().await;
            let detail = format!(
                "exec {cmd} timed out after {}s",
                timeout.as_secs_f64()
            );
            env.note_shell_outcome(&cmd, false, &detail);
            return ToolResult::fail(detail);
        }
        _ = &mut cancelled => {
            let _ = child.kill().await;
            return ToolResult::fail("interrupted by user");
        }
    };

    let mut decoded = String::from_utf8_lossy(&stdout_output).into_owned();
    if !stderr_output.is_empty() {
        if !decoded.is_empty() && !decoded.ends_with('\n') {
            decoded.push('\n');
        }
        decoded.push_str(&String::from_utf8_lossy(&stderr_output));
    }
    let total_lines = decoded.lines().count();
    let mut out_lines: Vec<String> = decoded
        .lines()
        .take(MAX_LINES + 50)
        .map(str::to_owned)
        .collect();
    out_lines = if crate::safety::is_directory_heavy(&cmd) {
        crate::safety::filter_directory_output(&out_lines, MAX_LINES)
    } else if total_lines > MAX_LINES {
        let n = total_lines - MAX_LINES;
        out_lines.truncate(MAX_LINES);
        out_lines.push(String::new());
        out_lines.push(format!("... and {n} more lines"));
        out_lines
    } else {
        out_lines
    };

    let mut body = out_lines.join("\n");
    if output_limited {
        body.push_str(&format!(
            "\n... output exceeded {MAX_OUTPUT_BYTES} bytes; process terminated"
        ));
        env.note_shell_outcome(&cmd, false, &body);
        return ToolResult::fail(body);
    }
    if !status.success() {
        let detail = format!("exit {}: {body}", status.code().unwrap_or(-1));
        env.note_shell_outcome(&cmd, false, &detail);
        return ToolResult::fail(detail);
    }
    let content = if body.trim().is_empty() {
        "(empty)".to_string()
    } else {
        body
    };
    env.note_shell_outcome(&cmd, true, &content);
    ToolResult::ok(content)
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "shell",
            &shell_description(),
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
        // Full command — UI cards scroll; truncating here made long ssh/path
        // commands unreadable in the tool input pane.
        arg_str(args, "cmd").unwrap_or_default()
    }

    async fn run(&self, args: &serde_json::Value, env: &dyn ToolEnv) -> ToolResult {
        run_shell(args, env, Duration::from_secs(TIMEOUT_SECS)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::{ToolControl, ToolEvent};
    use crate::tool::Tool;
    use std::path::{Path, PathBuf};

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

    struct DenyEnv(PathBuf);

    struct RejectLocalExecutionEnv(PathBuf);

    #[async_trait::async_trait]
    impl ToolEnv for DenyEnv {
        fn project_root(&self) -> &Path {
            &self.0
        }

        async fn confirm(&self, _message: &str) -> bool {
            false
        }

        async fn emit(&self, _event: ToolEvent) {}
    }

    #[async_trait::async_trait]
    impl ToolEnv for RejectLocalExecutionEnv {
        fn project_root(&self) -> &Path {
            &self.0
        }

        async fn confirm(&self, _message: &str) -> bool {
            true
        }

        async fn preflight_local_execution(&self, _source: &str) -> Result<(), String> {
            Err("exploration_scope_violation: mainline path".into())
        }

        async fn emit(&self, _event: ToolEvent) {}
    }

    #[test]
    fn shell_schema_names_the_actual_shell_and_pixi_escape_hatch() {
        let desc = ShellTool.schema().function.description;
        let shell = if cfg!(target_os = "windows") {
            "PowerShell"
        } else {
            "POSIX sh"
        };
        assert!(desc.contains(shell), "shell family missing: {desc}");
        assert!(
            desc.contains("pixi"),
            "scientific env guidance missing: {desc}"
        );
    }

    #[test]
    fn shell_preview_keeps_long_commands_intact() {
        let cmd = format!(
            "ssh CPU3 'ls {} {}'",
            "/data/xzg_data/2026-07-07-Cerichardii-rnaseq/omics-pipelines/rnaseq/Snakefile",
            "/data/xzg_data/2026-07-07-Cerichardii-rnaseq/omics-pipelines/rnaseq/README.md"
        );
        assert!(
            cmd.len() > 150,
            "premise: command longer than old 150-char cap"
        );
        let preview = ShellTool.preview(&json!({ "cmd": cmd.clone() }));
        assert_eq!(preview, cmd);
    }

    #[tokio::test]
    async fn denied_dangerous_command_stops_the_model_batch() {
        let env = DenyEnv(std::env::current_dir().unwrap());
        let result = run_shell(
            &json!({ "cmd": "rm -rf generated-output" }),
            &env,
            Duration::from_secs(1),
        )
        .await;

        assert!(!result.success);
        assert_eq!(result.control, ToolControl::StopBatch);
    }

    #[tokio::test]
    async fn hard_local_execution_boundary_stops_before_spawning() {
        let env = RejectLocalExecutionEnv(std::env::current_dir().unwrap());
        let result = run_shell(
            &json!({ "cmd": "echo should-not-run" }),
            &env,
            Duration::from_secs(1),
        )
        .await;

        assert!(!result.success);
        assert!(result.content.contains("exploration_scope_violation"));
        assert_eq!(result.control, ToolControl::StopBatch);
    }

    #[tokio::test]
    async fn silent_child_timeout_covers_output_drain() {
        let env = TestEnv(std::env::current_dir().unwrap());
        let cmd = if cfg!(target_os = "windows") {
            "Start-Sleep -Seconds 1"
        } else {
            "exec sleep 1"
        };

        let result = tokio::time::timeout(
            Duration::from_secs(2),
            run_shell(&json!({ "cmd": cmd }), &env, Duration::from_millis(50)),
        )
        .await
        .expect("shell timeout should fire while stdout and stderr are silent");

        assert!(!result.success);
        assert!(result.content.contains("timed out"), "{}", result.content);
    }

    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn kills_a_single_line_at_the_byte_limit() {
        let env = TestEnv(std::env::current_dir().unwrap());
        let result = run_shell(
            &json!({ "cmd": "head -c 1052672 /dev/zero" }),
            &env,
            Duration::from_secs(2),
        )
        .await;

        assert!(!result.success);
        assert!(result.content.contains("output exceeded"));
        assert!(result.content.len() <= MAX_OUTPUT_BYTES + 100);
    }

    #[test]
    fn stdout_decoder_preserves_split_utf8() {
        let bytes = "文".as_bytes();
        let mut carry = Vec::new();
        assert_eq!(decode_stdout_chunk(&mut carry, &bytes[..1], false), "");
        assert_eq!(decode_stdout_chunk(&mut carry, &bytes[1..], false), "文");
        assert!(carry.is_empty());
    }
}
