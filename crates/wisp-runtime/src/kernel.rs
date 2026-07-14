//! Persistent worker client and versioned JSON-lines protocol for Python and R.

use crate::manager::{RuntimeKernel, RuntimeObject, RuntimeObjectList, RuntimeOutput};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::{ffi::OsString, path::Path, time::Duration};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout},
};

pub const PROTOCOL_VERSION: u32 = 1;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Default)]
pub struct KernelResp {
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
    pub interrupted: bool,
    pub wall_s: f64,
    pub cpu_s: f64,
    pub rss_kb: u64,
}

#[derive(Debug, Clone)]
pub struct KernelReady {
    pub pid: u32,
    pub version: String,
}

#[derive(Deserialize, Debug)]
struct ReadyFrame {
    #[serde(rename = "type")]
    kind: String,
    protocol: u32,
    language: String,
    pid: u32,
    version: String,
}

#[derive(Deserialize, Debug)]
struct StreamChunk {
    id: String,
    #[serde(default)]
    data: String,
}

#[derive(Deserialize, Debug, Default)]
struct RawResp {
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
    #[serde(default, alias = "peak_rss_kb")]
    rss_kb: u64,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct RawObjects {
    id: String,
    objects: Vec<RuntimeObject>,
    total_count: usize,
}

pub struct KernelClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    ready: KernelReady,
}

impl KernelClient {
    /// Spawn and handshake with `python <worker>` in the current directory.
    pub async fn spawn(python: &Path, worker: &Path, envs: &[(String, String)]) -> Result<Self> {
        let cwd = std::env::current_dir().context("resolve kernel working directory")?;
        Self::spawn_in(python, worker, envs, &cwd).await
    }

    /// Spawn and handshake with a worker rooted in the owning project.
    pub async fn spawn_in(
        python: &Path,
        worker: &Path,
        envs: &[(String, String)],
        cwd: &Path,
    ) -> Result<Self> {
        Self::spawn_command(
            python,
            &[worker.as_os_str().to_os_string()],
            envs,
            Some(cwd),
            "python",
        )
        .await
    }

    /// Spawn any attached local transport (direct process, `wsl.exe`, or
    /// `ssh`) and wait for the selected language's ready frame.
    pub async fn spawn_command(
        program: &Path,
        args: &[OsString],
        envs: &[(String, String)],
        cwd: Option<&Path>,
        expected_language: &str,
    ) -> Result<Self> {
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args);
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        cmd.envs(
            envs.iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        );
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        wisp_tools::process::hide_console_async(&mut cmd);
        let mut child = cmd
            .spawn()
            .map_err(|error| anyhow!("spawn runtime transport {}: {error}", program.display()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("no kernel stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("no kernel stdout"))?;
        let mut stdout = BufReader::new(stdout);
        let ready = match read_ready(&mut stdout, expected_language, STARTUP_TIMEOUT).await {
            Ok(ready) => ready,
            Err(error) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(error.context("kernel worker handshake"));
            }
        };
        Ok(Self {
            child,
            stdin,
            stdout,
            ready,
        })
    }

    pub fn ready(&self) -> &KernelReady {
        &self.ready
    }

    async fn execute_cell(
        &mut self,
        id: &str,
        code: &str,
        output: &RuntimeOutput,
    ) -> Result<KernelResp> {
        if let Some(status) = self.child.try_wait()? {
            bail!("kernel worker exited before execution ({status})");
        }
        let request = serde_json::json!({ "type": "execute", "id": id, "code": code });
        self.stdin.write_all(request.to_string().as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        read_response(&mut self.stdout, id, output).await
    }

    async fn inspect_objects(&mut self, id: &str) -> Result<RuntimeObjectList> {
        if let Some(status) = self.child.try_wait()? {
            bail!("kernel worker exited before inspection ({status})");
        }
        let request = serde_json::json!({ "type": "inspect", "id": id });
        self.stdin.write_all(request.to_string().as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        read_objects(&mut self.stdout, id).await
    }

    async fn shutdown_worker(&mut self) -> Result<()> {
        let _ = self.stdin.shutdown().await;
        match tokio::time::timeout(Duration::from_millis(750), self.child.wait()).await {
            Ok(status) => {
                status?;
            }
            Err(_) => {
                self.child.kill().await?;
                let _ = self.child.wait().await?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl RuntimeKernel for KernelClient {
    async fn execute(
        &mut self,
        id: &str,
        code: &str,
        output: &RuntimeOutput,
    ) -> Result<KernelResp> {
        self.execute_cell(id, code, output).await
    }

    async fn inspect(&mut self, id: &str) -> Result<RuntimeObjectList> {
        self.inspect_objects(id).await
    }

    fn try_wait(&mut self) -> Result<Option<String>> {
        self.child
            .try_wait()
            .map(|status| status.map(|status| status.to_string()))
            .map_err(Into::into)
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.shutdown_worker().await
    }
}

async fn read_ready<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    expected_language: &str,
    timeout: Duration,
) -> Result<KernelReady> {
    let frame = tokio::time::timeout(timeout, read_protocol_line(reader))
        .await
        .map_err(|_| anyhow!("timed out waiting for ready frame"))??;
    let kind = frame
        .get("type")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("startup frame is missing string field 'type'"))?;
    if kind == "startup_error" {
        let message = frame
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or("worker initialization failed");
        bail!("worker startup failed: {message}");
    }
    if kind != "ready" {
        bail!("expected ready frame, received '{kind}'");
    }
    let ready: ReadyFrame =
        serde_json::from_value(frame).context("malformed ready frame fields")?;
    debug_assert_eq!(ready.kind, "ready");
    if ready.protocol != PROTOCOL_VERSION {
        bail!(
            "worker protocol {} is incompatible with host protocol {}",
            ready.protocol,
            PROTOCOL_VERSION
        );
    }
    if ready.language != expected_language {
        bail!(
            "worker language '{}' does not match requested '{}'",
            ready.language,
            expected_language
        );
    }
    Ok(KernelReady {
        pid: ready.pid,
        version: ready.version,
    })
}

async fn read_response<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    request_id: &str,
    output: &RuntimeOutput,
) -> Result<KernelResp> {
    loop {
        let frame = read_protocol_line(reader).await?;
        let kind = frame
            .get("type")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("protocol frame is missing string field 'type'"))?;
        match kind {
            "stdout_chunk" => {
                let chunk: StreamChunk =
                    serde_json::from_value(frame).context("malformed stdout_chunk frame")?;
                if chunk.id != request_id {
                    bail!(
                        "stdout_chunk id '{}' does not match active request '{}'",
                        chunk.id,
                        request_id
                    );
                }
                output.stdout(chunk.data);
            }
            "result" => {
                let response: RawResp =
                    serde_json::from_value(frame).context("malformed result frame")?;
                if response.id != request_id {
                    bail!(
                        "result id '{}' does not match active request '{}'",
                        response.id,
                        request_id
                    );
                }
                return Ok(KernelResp {
                    stdout: response.stdout,
                    stderr: response.stderr,
                    error: response.error,
                    interrupted: response.interrupted,
                    wall_s: response.usage.wall_s,
                    cpu_s: response.usage.cpu_s,
                    rss_kb: response.usage.rss_kb,
                });
            }
            other => bail!("unexpected protocol frame '{other}' during execution"),
        }
    }
}

async fn read_objects<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    request_id: &str,
) -> Result<RuntimeObjectList> {
    let frame = read_protocol_line(reader).await?;
    let kind = frame
        .get("type")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("protocol frame is missing string field 'type'"))?;
    if kind != "objects" {
        bail!("unexpected protocol frame '{kind}' during inspection");
    }
    let response: RawObjects = serde_json::from_value(frame).context("malformed objects frame")?;
    if response.id != request_id {
        bail!(
            "objects id '{}' does not match active request '{}'",
            response.id,
            request_id
        );
    }
    Ok(RuntimeObjectList {
        objects: response.objects,
        total_count: response.total_count,
    })
}

async fn read_protocol_line<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<serde_json::Value> {
    let mut line = String::new();
    let read = reader.read_line(&mut line).await?;
    if read == 0 {
        bail!("kernel worker closed protocol stdout");
    }
    let line = line.trim();
    if line.is_empty() {
        bail!("kernel worker emitted an empty protocol frame");
    }
    serde_json::from_str(line).context("kernel worker emitted malformed JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::RuntimeEvent;
    use tokio::io::{duplex, AsyncWriteExt};
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn ready_handshake_accepts_protocol_one_python_and_r() {
        for (language, version) in [("python", "3.13.1"), ("r", "4.5.1")] {
            let (reader, mut writer) = duplex(1024);
            writer
                .write_all(
                    format!(
                        "{{\"type\":\"ready\",\"protocol\":1,\"language\":\"{language}\",\"pid\":42,\"version\":\"{version}\"}}\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            let ready = read_ready(
                &mut BufReader::new(reader),
                language,
                Duration::from_secs(1),
            )
            .await
            .unwrap();
            assert_eq!(ready.pid, 42);
            assert_eq!(ready.version, version);
        }
    }

    #[tokio::test]
    async fn ready_handshake_surfaces_worker_startup_errors() {
        let (reader, mut writer) = duplex(1024);
        writer
            .write_all(
                b"{\"type\":\"startup_error\",\"error\":\"R package 'jsonlite' is required\"}\n",
            )
            .await
            .unwrap();
        let error = read_ready(&mut BufReader::new(reader), "r", Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("jsonlite"));
    }

    #[tokio::test]
    async fn ready_handshake_rejects_wrong_version_language_and_malformed_json() {
        for (frame, expected) in [
            (
                "{\"type\":\"ready\",\"protocol\":2,\"language\":\"python\",\"pid\":1,\"version\":\"3\"}\n",
                "incompatible",
            ),
            (
                "{\"type\":\"ready\",\"protocol\":1,\"language\":\"r\",\"pid\":1,\"version\":\"4\"}\n",
                "does not match",
            ),
            ("not-json\n", "malformed JSON"),
        ] {
            let (reader, mut writer) = duplex(1024);
            writer.write_all(frame.as_bytes()).await.unwrap();
            let error = read_ready(
                &mut BufReader::new(reader),
                "python",
                Duration::from_secs(1),
            )
            .await
            .unwrap_err();
            assert!(error.to_string().contains(expected), "{error:#}");
        }
    }

    #[tokio::test]
    async fn ready_handshake_reports_eof_and_timeout() {
        let (reader, writer) = duplex(64);
        drop(writer);
        let eof = read_ready(
            &mut BufReader::new(reader),
            "python",
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert!(eof.to_string().contains("closed protocol stdout"));

        let (reader, _writer) = duplex(64);
        let timeout = read_ready(
            &mut BufReader::new(reader),
            "python",
            Duration::from_millis(10),
        )
        .await
        .unwrap_err();
        assert!(timeout.to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn execution_correlates_stream_and_result_ids() {
        let (reader, mut writer) = duplex(2048);
        writer
            .write_all(
                br#"{"type":"stdout_chunk","id":"cell-1","data":"loading\n"}
{"type":"result","id":"cell-1","stdout":"done\n","stderr":"","error":null,"usage":{"wall_s":0.2,"cpu_s":0.1,"rss_kb":123}}
"#,
            )
            .await
            .unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let output = RuntimeOutput::new(tx);
        let response = read_response(&mut BufReader::new(reader), "cell-1", &output)
            .await
            .unwrap();
        assert_eq!(response.stdout, "done\n");
        assert_eq!(response.rss_kb, 123);
        assert!(matches!(
            rx.recv().await,
            Some(RuntimeEvent::Stdout(chunk)) if chunk == "loading\n"
        ));
    }

    #[tokio::test]
    async fn execution_rejects_a_mismatched_request_id() {
        let (reader, mut writer) = duplex(1024);
        writer
            .write_all(
                b"{\"type\":\"result\",\"id\":\"other\",\"stdout\":\"\",\"stderr\":\"\",\"error\":null}\n",
            )
            .await
            .unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let error = read_response(
            &mut BufReader::new(reader),
            "cell-1",
            &RuntimeOutput::new(tx),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("does not match active request"));
    }

    #[tokio::test]
    async fn inspection_correlates_ids_and_deserializes_bounded_metadata() {
        let (reader, mut writer) = duplex(2048);
        writer
            .write_all(
                br#"{"type":"objects","id":"inspect-1","objects":[{"name":"counts","typeName":"DataFrame","summary":"12000000 x 48","sizeBytes":4294967296}],"totalCount":1}
"#,
            )
            .await
            .unwrap();
        let result = read_objects(&mut BufReader::new(reader), "inspect-1")
            .await
            .unwrap();
        assert_eq!(result.total_count, 1);
        assert_eq!(result.objects[0].name, "counts");
        assert_eq!(result.objects[0].type_name, "DataFrame");
        assert_eq!(result.objects[0].size_bytes, Some(4_294_967_296));
    }

    #[tokio::test]
    async fn inspection_rejects_a_mismatched_request_id() {
        let (reader, mut writer) = duplex(1024);
        writer
            .write_all(b"{\"type\":\"objects\",\"id\":\"other\",\"objects\":[],\"totalCount\":0}\n")
            .await
            .unwrap();
        let error = read_objects(&mut BufReader::new(reader), "inspect-1")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("does not match active request"));
    }
}
