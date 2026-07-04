//! Persistent Python kernel client — drives `kernel_worker.py` over its
//! JSON-per-line stdio protocol.

use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use wisp_tools::{ToolEnv, ToolEvent};

#[derive(Debug, Clone, Default)]
pub struct KernelResp {
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
    pub interrupted: bool,
    pub wall_s: f64,
    pub cpu_s: f64,
    pub peak_rss_kb: u64,
}

#[derive(Deserialize, Debug)]
struct StreamChunk {
    #[serde(default)]
    data: String,
}

#[derive(Deserialize, Debug, Default)]
struct RawResp {
    #[allow(dead_code)]
    id: String,
    #[serde(default)]
    stdout: String,
    #[serde(default)]
    stderr: String,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    interrupted: bool,
    #[serde(default)]
    usage: RawUsage,
}

#[derive(Deserialize, Debug, Default)]
struct RawUsage {
    #[serde(default)]
    wall_s: f64,
    #[serde(default)]
    cpu_s: f64,
    #[serde(default)]
    peak_rss_kb: u64,
}

pub struct KernelClient {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl KernelClient {
    /// Spawn the kernel worker with `python <worker>`.
    pub fn spawn(python: &Path, worker: &Path) -> Result<Self> {
        let mut cmd = tokio::process::Command::new(python);
        cmd.arg(worker);
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        wisp_tools::process::hide_console_async(&mut cmd);
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("spawn kernel worker: {e}"))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        // Reap the child when the client drops: leak intentionally — the
        // worker is long-lived for the session. (A Drop that kills it would
        // destroy the persistent namespace.)
        std::mem::forget(child);
        Ok(Self {
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    /// Execute one cell; stream `stdout_chunk` events to `env`, return the
    /// final response.
    pub async fn execute(&mut self, id: &str, code: &str, env: &dyn ToolEnv) -> Result<KernelResp> {
        let req = serde_json::json!({ "id": id, "code": code });
        self.stdin.write_all(req.to_string().as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;

        loop {
            // Cancel at line boundaries only: read_line is not cancellation-safe,
            // so we never wrap it in a timeout/select. We don't kill the worker
            // (its namespace is persistent) — we stop waiting; its late response
            // is skipped next cell by id mismatch. ponytail: a compute-bound cell
            // that emits no lines stays uninterruptible, same ceiling as shell.
            if env.is_cancelled() {
                return Err(anyhow!("interrupted by user"));
            }
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).await?;
            if n == 0 {
                return Err(anyhow!("kernel worker closed its stdout mid-cell"));
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let val: serde_json::Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if val.get("type").and_then(|t| t.as_str()) == Some("stdout_chunk") {
                if let Ok(chunk) = serde_json::from_value::<StreamChunk>(val) {
                    env.emit(ToolEvent::Stdout { chunk: chunk.data }).await;
                }
                continue;
            }
            if val.get("id").and_then(|t| t.as_str()) == Some(id) {
                let raw: RawResp = serde_json::from_value(val)?;
                return Ok(KernelResp {
                    stdout: raw.stdout,
                    stderr: raw.stderr,
                    error: raw.error,
                    interrupted: raw.interrupted,
                    wall_s: raw.usage.wall_s,
                    cpu_s: raw.usage.cpu_s,
                    peak_rss_kb: raw.usage.peak_rss_kb,
                });
            }
        }
    }
}
