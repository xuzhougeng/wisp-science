//! Persistent worker client and versioned JSON-lines protocol for Python and R.

use crate::manager::{RuntimeKernel, RuntimeObject, RuntimeObjectList, RuntimeOutput};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::{ffi::OsString, path::Path, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStderr, ChildStdin, ChildStdout},
    sync::Mutex,
    task::JoinHandle,
};
use wisp_tools::process::ProcessTree;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_CODE_BYTES: usize = 1024 * 1024;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_WORKER_STDERR_BYTES: usize = 32 * 1024;
/// Stopping a worker is bounded by these three budgets in sequence. Every step
/// has a way to overrun on a real host: a worker can ignore stdin EOF, a
/// wrapper process can outlive its own kill, and a descendant that inherited
/// the stderr pipe can hold its write end open indefinitely.
const WORKER_EXIT_GRACE: Duration = Duration::from_secs(2);
const WORKER_KILL_WAIT: Duration = Duration::from_secs(2);
const WORKER_STDERR_DRAIN: Duration = Duration::from_millis(500);

type WorkerStderrTail = Arc<Mutex<Vec<u8>>>;

#[derive(Debug, Clone, Default)]
pub struct KernelResp {
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
    pub interrupted: bool,
    pub wall_s: f64,
    pub cpu_s: f64,
    pub rss_kb: u64,
    /// Absolute paths the worker observed this cell write. `None` means the
    /// worker did not (or could not) observe; `Some([])` means it observed
    /// and the cell wrote nothing. Never conflate the two.
    pub files_written: Option<Vec<String>>,
    /// `project` means `files_written` contains project-relative paths from a
    /// host-configured write scope. Absent keeps the legacy absolute-path
    /// contract for older bundled workers.
    pub files_written_base: Option<String>,
    /// Base64-encoded PNG snapshots of the plots this cell produced, oldest
    /// first. Empty when the cell drew nothing or the worker cannot capture
    /// (older bundled workers omit the field entirely).
    pub plots: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelWriteScope {
    pub root: String,
    pub skip_dirs: Vec<String>,
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
    #[serde(default)]
    files_written: Option<Vec<String>>,
    #[serde(default)]
    files_written_base: Option<String>,
    #[serde(default)]
    plots: Vec<String>,
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
    /// Held as `Option` so shutdown can drop the handle. Closing the write end
    /// is the only way the worker ever sees stdin EOF: tokio's
    /// `ChildStdin::shutdown` is a no-op on both Unix and Windows.
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr_tail: WorkerStderrTail,
    stderr_task: Option<JoinHandle<()>>,
    /// Termination boundary covering the worker and everything it starts. An
    /// interpreter is frequently a launcher rather than the real process:
    /// Windows `Rscript.exe` re-launches `bin\x64\Rscript.exe` through
    /// `cmd.exe`, and any cell can spawn background children of its own.
    process_tree: ProcessTree,
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
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        // Sets CREATE_NO_WINDOW itself, so this replaces `hide_console_async`.
        ProcessTree::configure(&mut cmd);
        let mut child = cmd
            .spawn()
            .map_err(|error| anyhow!("spawn runtime transport {}: {error}", program.display()))?;
        let process_tree = ProcessTree::attach(&child).map_err(|error| {
            let _ = child.start_kill();
            anyhow!("attach runtime worker process tree: {error}")
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("no kernel stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("no kernel stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("no kernel stderr"))?;
        let (stderr_tail, stderr_task) = capture_worker_stderr(stderr);
        let mut stdout = BufReader::new(stdout);
        let ready = match read_ready(&mut stdout, expected_language, STARTUP_TIMEOUT).await {
            Ok(ready) => ready,
            Err(error) => {
                drop(stdin);
                let status = kill_worker(&mut child, &process_tree).await.ok();
                drain_stderr(stderr_task).await;
                let detail = worker_failure_detail(status.as_ref(), &stderr_tail).await;
                return Err(anyhow!("kernel worker handshake: {error}; {detail}"));
            }
        };
        Ok(Self {
            child,
            stdin: Some(stdin),
            stdout,
            stderr_tail,
            stderr_task: Some(stderr_task),
            process_tree,
            ready,
        })
    }

    pub fn ready(&self) -> &KernelReady {
        &self.ready
    }

    /// Configure the worker's project write boundary before its first cell.
    /// Only Python workers currently implement this optional startup frame.
    pub async fn configure_write_scope(&mut self, scope: &KernelWriteScope) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        self.send(serde_json::json!({
            "type": "configure",
            "id": id,
            "write_scope": {
                "root": scope.root,
                "skip_dirs": scope.skip_dirs,
            }
        }))
        .await?;
        read_configured(&mut self.stdout, &id, STARTUP_TIMEOUT).await
    }

    async fn execute_cell(
        &mut self,
        id: &str,
        code: &str,
        output: &RuntimeOutput,
    ) -> Result<KernelResp> {
        if code.len() > MAX_CODE_BYTES {
            bail!("runtime code exceeds {MAX_CODE_BYTES} byte limit");
        }
        if let Some(status) = self.child.try_wait()? {
            self.finish_stderr_capture().await;
            let detail = worker_failure_detail(Some(&status), &self.stderr_tail).await;
            bail!("kernel worker exited before execution request '{id}'; {detail}");
        }
        self.send(serde_json::json!({ "type": "execute", "id": id, "code": code }))
            .await?;
        match read_response(&mut self.stdout, id, output).await {
            Ok(response) => Ok(response),
            Err(error) => {
                let detail = self
                    .failure_detail(&format!("execution request '{id}'"))
                    .await;
                Err(anyhow!("{error}; {detail}"))
            }
        }
    }

    async fn inspect_objects(&mut self, id: &str) -> Result<RuntimeObjectList> {
        if let Some(status) = self.child.try_wait()? {
            self.finish_stderr_capture().await;
            let detail = worker_failure_detail(Some(&status), &self.stderr_tail).await;
            bail!("kernel worker exited before inspection request '{id}'; {detail}");
        }
        self.send(serde_json::json!({ "type": "inspect", "id": id }))
            .await?;
        match read_objects(&mut self.stdout, id).await {
            Ok(objects) => Ok(objects),
            Err(error) => {
                let detail = self
                    .failure_detail(&format!("inspection request '{id}'"))
                    .await;
                Err(anyhow!("{error}; {detail}"))
            }
        }
    }

    async fn send(&mut self, request: serde_json::Value) -> Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow!("kernel worker stdin is already closed"))?;
        stdin.write_all(request.to_string().as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Stop the worker within a bounded budget. Dropping stdin gives it EOF so
    /// it can exit on its own; anything slower is killed. Either way the stop
    /// reclaims the whole tree, so neither a launcher's real interpreter nor a
    /// background process some cell started can outlive the runtime.
    async fn shutdown_worker(&mut self) -> Result<()> {
        drop(self.stdin.take());
        let stopped = match tokio::time::timeout(WORKER_EXIT_GRACE, self.child.wait()).await {
            Ok(status) => status.map(|_| ()).map_err(anyhow::Error::from),
            Err(_) => kill_worker(&mut self.child, &self.process_tree)
                .await
                .map(|_| ()),
        };
        let _ = self.process_tree.terminate_if_running();
        self.finish_stderr_capture().await;
        stopped
    }

    async fn failure_detail(&mut self, action: &str) -> String {
        tokio::task::yield_now().await;
        let status = self.child.try_wait().ok().flatten();
        if status.is_some() {
            self.finish_stderr_capture().await;
        }
        format!(
            "worker failed during {action}; {}",
            worker_failure_detail(status.as_ref(), &self.stderr_tail).await
        )
    }

    async fn finish_stderr_capture(&mut self) {
        if let Some(task) = self.stderr_task.take() {
            drain_stderr(task).await;
        }
    }
}

/// Terminate the worker's whole process tree, then reap the direct child under
/// a deadline. Killing only the direct child would leave the real interpreter
/// running whenever it was started through a launcher: on Windows
/// `Rscript.exe` re-launches `bin\x64\Rscript.exe` through `cmd.exe`.
///
/// The tree is signalled before the child is reaped, which is what keeps a
/// freed Unix process-group id from being signalled after reuse.
async fn kill_worker(child: &mut Child, tree: &ProcessTree) -> Result<std::process::ExitStatus> {
    tree.terminate_forcefully()?;
    tokio::time::timeout(WORKER_KILL_WAIT, child.wait())
        .await
        .map_err(|_| anyhow!("kernel worker did not exit within {WORKER_KILL_WAIT:?} of kill"))?
        .map_err(Into::into)
}

/// Collect whatever stderr the worker already produced, then stop waiting. The
/// write end of that pipe is inherited by every descendant the worker started,
/// so EOF can arrive long after the worker itself is gone — or never.
async fn drain_stderr(task: JoinHandle<()>) {
    let abort = task.abort_handle();
    if tokio::time::timeout(WORKER_STDERR_DRAIN, task)
        .await
        .is_err()
    {
        abort.abort();
    }
}

fn capture_worker_stderr(stderr: ChildStderr) -> (WorkerStderrTail, JoinHandle<()>) {
    let tail = Arc::new(Mutex::new(Vec::with_capacity(MAX_WORKER_STDERR_BYTES)));
    let task_tail = tail.clone();
    let task = tokio::spawn(async move {
        let mut stderr = stderr;
        let mut chunk = [0_u8; 4096];
        loop {
            let Ok(read) = stderr.read(&mut chunk).await else {
                break;
            };
            if read == 0 {
                break;
            }
            let mut tail = task_tail.lock().await;
            append_bounded_tail(&mut tail, &chunk[..read]);
        }
    });
    (tail, task)
}

fn append_bounded_tail(tail: &mut Vec<u8>, chunk: &[u8]) {
    if chunk.len() >= MAX_WORKER_STDERR_BYTES {
        tail.clear();
        tail.extend_from_slice(&chunk[chunk.len() - MAX_WORKER_STDERR_BYTES..]);
        return;
    }
    let overflow = (tail.len() + chunk.len()).saturating_sub(MAX_WORKER_STDERR_BYTES);
    if overflow > 0 {
        tail.drain(..overflow);
    }
    tail.extend_from_slice(chunk);
}

async fn worker_failure_detail(
    status: Option<&std::process::ExitStatus>,
    stderr_tail: &WorkerStderrTail,
) -> String {
    let status = status
        .map(|status| format!("exit status {status}"))
        .unwrap_or_else(|| "exit status unavailable".into());
    let stderr = String::from_utf8_lossy(&stderr_tail.lock().await)
        .trim()
        .to_string();
    if stderr.is_empty() {
        format!("{status}; worker stderr was empty")
    } else {
        format!("{status}; worker stderr tail: {stderr}")
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
                    files_written: response.files_written,
                    files_written_base: response.files_written_base,
                    plots: response.plots,
                });
            }
            other => bail!("unexpected protocol frame '{other}' during execution"),
        }
    }
}

async fn read_configured<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    request_id: &str,
    timeout: Duration,
) -> Result<()> {
    let frame = tokio::time::timeout(timeout, read_protocol_line(reader))
        .await
        .map_err(|_| anyhow!("timed out waiting for write-scope configuration"))??;
    let kind = frame
        .get("type")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("write-scope response is missing string field 'type'"))?;
    let id = frame
        .get("id")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    if id != request_id {
        bail!(
            "write-scope response id '{}' does not match active request '{}'",
            id,
            request_id
        );
    }
    match kind {
        "configured" => Ok(()),
        "configure_error" => bail!(
            "kernel worker rejected write scope: {}",
            frame
                .get("error")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown configuration error")
        ),
        other => bail!("unexpected protocol frame '{other}' during write-scope configuration"),
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
    async fn write_scope_configuration_accepts_ack_and_surfaces_rejection() {
        let (reader, mut writer) = duplex(1024);
        writer
            .write_all(b"{\"type\":\"configured\",\"id\":\"scope-1\"}\n")
            .await
            .unwrap();
        read_configured(
            &mut BufReader::new(reader),
            "scope-1",
            Duration::from_secs(1),
        )
        .await
        .unwrap();

        let (reader, mut writer) = duplex(1024);
        writer
            .write_all(
                b"{\"type\":\"configure_error\",\"id\":\"scope-2\",\"error\":\"bad root\"}\n",
            )
            .await
            .unwrap();
        let error = read_configured(
            &mut BufReader::new(reader),
            "scope-2",
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("bad root"));
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
    async fn worker_handshake_failure_includes_bounded_stderr_and_exit_status() {
        #[cfg(target_os = "windows")]
        let (program, args) = (
            Path::new("cmd.exe"),
            vec![
                OsString::from("/C"),
                OsString::from("echo native runtime crash 1>&2 & exit /B 23"),
            ],
        );
        #[cfg(not(target_os = "windows"))]
        let (program, args) = (
            Path::new("sh"),
            vec![
                OsString::from("-c"),
                OsString::from("printf 'native runtime crash\\n' >&2; exit 23"),
            ],
        );

        let error = match KernelClient::spawn_command(program, &args, &[], None, "python").await {
            Ok(_) => panic!("crashing worker unexpectedly completed its handshake"),
            Err(error) => error,
        };
        let detail = error.to_string();
        assert!(detail.contains("closed protocol stdout"), "{detail}");
        assert!(detail.contains("native runtime crash"), "{detail}");
        assert!(detail.contains("23"), "{detail}");
    }

    const READY_FRAME: &str =
        r#"{"type":"ready","protocol":1,"language":"python","pid":1,"version":"test"}"#;

    /// A worker that completes its handshake and then behaves like `tail`.
    #[cfg(unix)]
    fn fake_worker(tail: &str) -> Vec<OsString> {
        vec![
            OsString::from("-c"),
            OsString::from(format!("printf '%s\\n' '{READY_FRAME}'; {tail}")),
        ]
    }

    /// Dropping the handle is the only thing that closes the pipe: tokio's
    /// `ChildStdin::poll_shutdown` returns `Ready(Ok(()))` without touching the
    /// file descriptor on both Unix and Windows, so a worker waiting on stdin
    /// used to be killed on every stop instead of exiting on its own.
    #[cfg(unix)]
    #[tokio::test]
    async fn closing_stdin_lets_a_worker_exit_before_the_kill_deadline() {
        let mut client = KernelClient::spawn_command(
            Path::new("sh"),
            &fake_worker("cat > /dev/null"),
            &[],
            None,
            "python",
        )
        .await
        .unwrap();

        let started = std::time::Instant::now();
        client.shutdown().await.unwrap();
        assert!(
            started.elapsed() < WORKER_EXIT_GRACE,
            "worker should exit on stdin EOF rather than wait out the grace period"
        );
    }

    /// Descendants inherit the stderr pipe, so its EOF can arrive long after
    /// the worker is gone. `tests/worker_process_tree.rs` covers the process
    /// tree itself on every OS; this only pins the drain budget, which is the
    /// backstop for a descendant that escapes the termination boundary.
    #[tokio::test(start_paused = true)]
    async fn stderr_drain_gives_up_on_a_reader_that_never_reaches_eof() {
        let started = tokio::time::Instant::now();
        drain_stderr(tokio::spawn(std::future::pending())).await;
        assert!(started.elapsed() >= WORKER_STDERR_DRAIN);
    }

    #[test]
    fn worker_stderr_tail_is_bounded_and_keeps_the_latest_bytes() {
        let mut tail = Vec::new();
        append_bounded_tail(&mut tail, &vec![b'x'; MAX_WORKER_STDERR_BYTES + 17]);
        append_bounded_tail(&mut tail, b"CRASH_END");
        assert_eq!(tail.len(), MAX_WORKER_STDERR_BYTES);
        assert!(tail.ends_with(b"CRASH_END"));
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
        assert_eq!(response.files_written, None);
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
    async fn result_frame_round_trips_files_written() {
        let (reader, mut writer) = duplex(2048);
        writer
            .write_all(
                br#"{"type":"result","id":"cell-1","stdout":"","stderr":"","error":null,"files_written":["a.txt","b.txt"],"files_written_base":"project"}
"#,
            )
            .await
            .unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let response = read_response(
            &mut BufReader::new(reader),
            "cell-1",
            &RuntimeOutput::new(tx),
        )
        .await
        .unwrap();
        assert_eq!(
            response.files_written,
            Some(vec!["a.txt".into(), "b.txt".into()])
        );
        assert_eq!(response.files_written_base.as_deref(), Some("project"));
    }

    #[tokio::test]
    async fn result_frame_round_trips_plots_and_defaults_to_empty() {
        let (reader, mut writer) = duplex(2048);
        writer
            .write_all(
                br#"{"type":"result","id":"cell-1","stdout":"","stderr":"","error":null,"plots":["aGVsbG8=","d29ybGQ="]}
"#,
            )
            .await
            .unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let response = read_response(
            &mut BufReader::new(reader),
            "cell-1",
            &RuntimeOutput::new(tx),
        )
        .await
        .unwrap();
        assert_eq!(
            response.plots,
            vec!["aGVsbG8=".to_string(), "d29ybGQ=".to_string()]
        );

        // Older workers omit the field: the response still parses, plot-free.
        let (reader, mut writer) = duplex(1024);
        writer
            .write_all(
                b"{\"type\":\"result\",\"id\":\"cell-1\",\"stdout\":\"\",\"stderr\":\"\",\"error\":null}\n",
            )
            .await
            .unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let response = read_response(
            &mut BufReader::new(reader),
            "cell-1",
            &RuntimeOutput::new(tx),
        )
        .await
        .unwrap();
        assert!(response.plots.is_empty());
    }

    #[tokio::test]
    async fn result_frame_without_files_written_is_none() {
        let (reader, mut writer) = duplex(1024);
        writer
            .write_all(
                b"{\"type\":\"result\",\"id\":\"cell-1\",\"stdout\":\"\",\"stderr\":\"\",\"error\":null}\n",
            )
            .await
            .unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let response = read_response(
            &mut BufReader::new(reader),
            "cell-1",
            &RuntimeOutput::new(tx),
        )
        .await
        .unwrap();
        assert_eq!(response.files_written, None);
    }

    #[tokio::test]
    async fn result_frame_with_empty_files_written_is_some_empty() {
        let (reader, mut writer) = duplex(1024);
        writer
            .write_all(
                b"{\"type\":\"result\",\"id\":\"cell-1\",\"stdout\":\"\",\"stderr\":\"\",\"error\":null,\"files_written\":[]}\n",
            )
            .await
            .unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let response = read_response(
            &mut BufReader::new(reader),
            "cell-1",
            &RuntimeOutput::new(tx),
        )
        .await
        .unwrap();
        assert_eq!(response.files_written, Some(Vec::new()));
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
