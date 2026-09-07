use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::Mutex;

mod cleanup;
mod harvest_remote;
mod local_detached;
mod remote;
pub mod remote_files;
mod tools;
mod transfer;

pub use harvest_remote::WorkspaceListing;
#[cfg(all(test, windows))]
use remote::scp_local_path;
#[cfg(test)]
use remote::{cancel_payload, launch_payload, poll_payload, prepare_payload};
use remote::{
    cancel_remote, checked_output, ensure_remote_started, permanent_remote_start_error,
    poll_remote, prepare_remote, remote_poll_interval, remote_poll_interval_for,
    remote_terminal_status, resolve_input_paths, ssh_dedicated_script_command, ssh_script_command,
    PrepareRemote, RemoteCancel, RemotePollState,
};
#[cfg(test)]
use remote::{parse_input_progress, remote_poll_delay_secs};
pub use tools::{
    CancelRunTool, CleanupRunWorkspaceTool, GetRunTool, HarvestRunTool, ListRemoteFilesTool,
    MonitorRunTool, RemoveRemoteFilesTool, RunInContextTool,
};
pub use transfer::{
    load_trust_edges, persist_transfer_handle, revoke_trust_edge, submit_local_uploads_to_context,
    RevokeTrustResponse, SshTrustEdge, TransferHandle, UploadToContextItem,
};
pub use transfer::{ConfigureSshTrustTool, TransferBetweenContextsTool};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitRunRequest {
    pub context_id: String,
    pub command: String,
    pub title: Option<String>,
    pub timeout_secs: Option<u64>,
    /// Project-relative files copied into an SSH run's remote workdir.
    #[serde(default)]
    pub input_paths: Option<Vec<String>>,
    #[serde(default)]
    pub output_specs: Option<Vec<crate::harvest::OutputSpec>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitRunResponse {
    pub run_id: String,
    pub status: wisp_store::RunStatus,
    pub exit_code: Option<i64>,
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
    pub remote_workdir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunPreflightStatus {
    Passed,
    Warning,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunPreflightCheck {
    pub name: String,
    pub status: RunPreflightStatus,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunPreflightReport {
    pub status: RunPreflightStatus,
    pub context_id: String,
    pub language: String,
    pub checks: Vec<RunPreflightCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunPreflightSpec {
    pub language: String,
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub syntax_paths: Vec<String>,
    #[serde(default)]
    pub allow_warnings: bool,
}

impl RunPreflightReport {
    fn new(context_id: &str, language: &str) -> Self {
        Self {
            status: RunPreflightStatus::Passed,
            context_id: context_id.into(),
            language: language.into(),
            checks: Vec::new(),
        }
    }

    fn push(&mut self, name: impl Into<String>, status: RunPreflightStatus, detail: String) {
        if status == RunPreflightStatus::Failed {
            self.status = RunPreflightStatus::Failed;
        } else if status == RunPreflightStatus::Warning && self.status == RunPreflightStatus::Passed
        {
            self.status = RunPreflightStatus::Warning;
        }
        self.checks.push(RunPreflightCheck {
            name: name.into(),
            status,
            detail,
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCommand {
    pub context_id: String,
    pub program: String,
    pub args: Vec<String>,
    pub script: String,
    pub cwd: Option<PathBuf>,
    pub stdin: Option<String>,
    /// Extra process environment (e.g. SSH_ASKPASS for password auth).
    pub envs: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCommandOutput {
    pub exit_code: i64,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutputUpdate {
    pub stream: RunOutputStream,
    pub chunk: Vec<u8>,
}

pub const PUBLICATION_REPRODUCTION_CONTEXT_ID: &str = "publication-reproduction";

pub fn run_environment_snapshot(context: &wisp_store::ExecutionContext) -> serde_json::Value {
    let process = ["LANG", "LC_ALL", "TZ"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok().map(|value| (name, value)))
        .collect::<std::collections::BTreeMap<_, _>>();
    serde_json::json!({
        "schema_version": 2,
        "context": {
            "id": context.id,
            "kind": context.kind,
            "config": serde_json::from_str::<serde_json::Value>(&context.config_json)
                .unwrap_or_default(),
            "capabilities": serde_json::from_str::<serde_json::Value>(
                &context.capabilities_json,
            )
            .unwrap_or_default(),
        },
        "process": process,
        "wisp_host": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        },
    })
}

#[async_trait::async_trait]
pub trait RunCommandRunner: Send + Sync {
    async fn run(&self, command: RunCommand, timeout: Duration)
        -> Result<RunCommandOutput, String>;

    async fn run_streaming(
        &self,
        command: RunCommand,
        timeout: Duration,
        _updates: tokio::sync::mpsc::UnboundedSender<RunOutputUpdate>,
    ) -> Result<RunCommandOutput, String> {
        self.run(command, timeout).await
    }
}

#[derive(Clone)]
pub struct ProcessRunRunner;

const MAX_RUN_OUTPUT_BYTES: usize = 64 * 1024;

struct SshAuthEnvCleanup(Vec<(String, String)>);

impl Drop for SshAuthEnvCleanup {
    fn drop(&mut self) {
        crate::ssh_hosts::cleanup_password_auth_env(&self.0);
    }
}

fn transfer_progress(
    direction: &str,
    phase: &str,
    completed_bytes: u64,
    total_bytes: u64,
    files_completed: u64,
    files_total: u64,
    current_file: Option<String>,
    started: Instant,
) -> wisp_store::RunProgress {
    let elapsed = started.elapsed();
    let bytes_per_second = (elapsed >= Duration::from_secs(1))
        .then(|| (completed_bytes as f64 / elapsed.as_secs_f64()) as u64)
        .filter(|rate| *rate > 0);
    let eta_seconds = bytes_per_second
        .filter(|_| completed_bytes < total_bytes)
        .map(|rate| total_bytes.saturating_sub(completed_bytes).div_ceil(rate));
    wisp_store::RunProgress {
        phase: phase.into(),
        direction: direction.into(),
        completed_bytes: completed_bytes.min(total_bytes),
        total_bytes,
        files_completed: files_completed.min(files_total),
        files_total,
        current_file,
        bytes_per_second,
        eta_seconds,
        updated_at: chrono::Utc::now().timestamp(),
    }
}

fn append_tail_bytes(tail: &mut Vec<u8>, chunk: &[u8]) {
    if chunk.len() >= MAX_RUN_OUTPUT_BYTES {
        tail.clear();
        tail.extend_from_slice(&chunk[chunk.len() - MAX_RUN_OUTPUT_BYTES..]);
        return;
    }
    let overflow = (tail.len() + chunk.len()).saturating_sub(MAX_RUN_OUTPUT_BYTES);
    if overflow > 0 {
        tail.drain(..overflow);
    }
    tail.extend_from_slice(chunk);
}

async fn read_tail<R: AsyncRead + Unpin>(
    mut reader: R,
    stream: RunOutputStream,
    updates: Option<tokio::sync::mpsc::UnboundedSender<RunOutputUpdate>>,
) -> std::io::Result<Vec<u8>> {
    let mut tail = Vec::with_capacity(MAX_RUN_OUTPUT_BYTES);
    let mut chunk = [0_u8; 8192];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            return Ok(tail);
        }
        append_tail_bytes(&mut tail, &chunk[..read]);
        if let Some(updates) = &updates {
            let _ = updates.send(RunOutputUpdate {
                stream,
                chunk: chunk[..read].to_vec(),
            });
        }
    }
}

fn is_ssh_transport_program(program: &str) -> bool {
    let name = std::path::Path::new(program)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(program)
        .to_ascii_lowercase();
    matches!(
        name.as_str(),
        "ssh" | "scp" | "sftp" | "ssh.exe" | "scp.exe" | "sftp.exe"
    )
}

fn identity_file_from_args(args: &[String]) -> Option<&str> {
    args.windows(2)
        .find_map(|pair| (pair[0] == "-i").then_some(pair[1].as_str()))
}

fn record_ssh_runner_outcome(context_id: &str, result: &Result<RunCommandOutput, String>) {
    match result {
        Ok(output) if output.exit_code == 0 => {
            crate::ssh_guard::record_success(context_id);
        }
        Ok(output) => {
            let detail = if output.stderr.trim().is_empty() {
                output.stdout.trim()
            } else {
                output.stderr.trim()
            };
            if crate::ssh_guard::is_authentication_failure(detail) {
                crate::ssh_guard::record_failure(context_id, detail);
            }
        }
        Err(error) => {
            if crate::ssh_guard::is_authentication_failure(error) {
                crate::ssh_guard::record_failure(context_id, error);
            }
        }
    }
}

#[async_trait::async_trait]
impl RunCommandRunner for ProcessRunRunner {
    async fn run(
        &self,
        command: RunCommand,
        timeout: Duration,
    ) -> Result<RunCommandOutput, String> {
        run_process(command, timeout, None).await
    }

    async fn run_streaming(
        &self,
        command: RunCommand,
        timeout: Duration,
        updates: tokio::sync::mpsc::UnboundedSender<RunOutputUpdate>,
    ) -> Result<RunCommandOutput, String> {
        run_process(command, timeout, Some(updates)).await
    }
}

async fn run_process(
    command: RunCommand,
    timeout: Duration,
    updates: Option<tokio::sync::mpsc::UnboundedSender<RunOutputUpdate>>,
) -> Result<RunCommandOutput, String> {
    // Process futures are dropped on Run cancellation. Keep password
    // passfile cleanup RAII-based so cancellation cannot leave a secret.
    let _auth_cleanup = SshAuthEnvCleanup(command.envs.clone());
    let ssh_transport =
        is_ssh_transport_program(&command.program) || command.context_id.starts_with("ssh:");
    if ssh_transport {
        crate::ssh_guard::assert_allowed(&command.context_id)?;
        if let Some(path) = identity_file_from_args(&command.args) {
            if let Err(error) = crate::ssh_hosts::ensure_identity_path_accessible(path) {
                crate::ssh_guard::record_failure(&command.context_id, &error);
                return Err(error);
            }
        }
        if let Some(payload) = crate::ssh_master::eligible_payload(
            &command.program,
            &command.args,
            command.stdin.as_deref(),
        ) {
            let ssh_args = command.args[..command.args.len() - 1].to_vec();
            let result = crate::ssh_master::run(
                &command.context_id,
                ssh_args,
                &command.envs,
                payload,
                timeout,
            )
            .await
            .map(|output| RunCommandOutput {
                exit_code: output.exit_code,
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: output.stderr,
            });
            record_ssh_runner_outcome(&command.context_id, &result);
            return result;
        }
    }
    let mut cmd = Command::new(&command.program);
    if command.context_id == PUBLICATION_REPRODUCTION_CONTEXT_ID {
        cmd.env_clear();
    }
    cmd.args(&command.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if !command.envs.is_empty() {
        cmd.envs(command.envs.iter().cloned());
    }
    if command.stdin.is_some() {
        cmd.stdin(Stdio::piped());
    }
    if let Some(cwd) = &command.cwd {
        cmd.current_dir(cwd);
    }
    wisp_tools::process::hide_console_async(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn {}: {e}", command.program))?;
    let program = command.program.clone();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("failed to open {program} stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("failed to open {program} stderr"))?;
    let mut stdout_task = tokio::spawn(read_tail(stdout, RunOutputStream::Stdout, updates.clone()));
    let mut stderr_task = tokio::spawn(read_tail(stderr, RunOutputStream::Stderr, updates));
    let input = command.stdin;
    let operation = async {
        if let Some(input) = input {
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| format!("failed to open {program} stdin"))?;
            stdin
                .write_all(input.as_bytes())
                .await
                .map_err(|e| format!("failed to write {program} stdin: {e}"))?;
            stdin
                .shutdown()
                .await
                .map_err(|e| format!("failed to close {program} stdin: {e}"))?;
        }
        let status = child
            .wait()
            .await
            .map_err(|e| format!("run_in_context wait failed: {e}"))?;
        let stdout = (&mut stdout_task)
            .await
            .map_err(|e| format!("run_in_context stdout task failed: {e}"))?
            .map_err(|e| format!("run_in_context stdout read failed: {e}"))?;
        let stderr = (&mut stderr_task)
            .await
            .map_err(|e| format!("run_in_context stderr task failed: {e}"))?
            .map_err(|e| format!("run_in_context stderr read failed: {e}"))?;
        Ok::<_, String>((status, stdout, stderr))
    };
    let result = match tokio::time::timeout(timeout, operation).await {
        Ok(Ok((status, stdout, stderr))) => Ok(RunCommandOutput {
            exit_code: status.code().unwrap_or(-1) as i64,
            stdout: String::from_utf8_lossy(&stdout).to_string(),
            stderr: String::from_utf8_lossy(&stderr).to_string(),
        }),
        Ok(Err(error)) => {
            stdout_task.abort();
            stderr_task.abort();
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            Err(error)
        }
        Err(_) => {
            stdout_task.abort();
            stderr_task.abort();
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            Err(format!(
                "run_in_context timed out after {}s",
                timeout.as_secs()
            ))
        }
    };
    if ssh_transport {
        record_ssh_runner_outcome(&command.context_id, &result);
    }
    result
}

const REMOTE_RPC_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LocalTransport {
    Posix {
        context_id: String,
        program: String,
        /// Full argv after `program`, including the shell entrypoint
        /// (e.g. `["-s"]` locally or `["-d", "Ubuntu", "--", "sh", "-s"]` for WSL).
        args: Vec<String>,
    },
    Windows {
        context_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RemoteRunHandle {
    SshDirect {
        connection: crate::ssh_hosts::SshConnection,
        workdir: String,
        token: String,
        #[serde(default)]
        inputs_staged: bool,
        pgid: Option<i64>,
        start_time: Option<u64>,
    },
    LocalDetached {
        transport: LocalTransport,
        workdir: String,
        token: String,
        #[serde(default)]
        inputs_staged: bool,
        pgid: Option<i64>,
        /// Process start identity: /proc starttime, macOS lstart, or Windows CreationDate.
        start_identity: Option<String>,
        /// Absolute path for command cwd; None keeps the transport default.
        command_cwd: Option<String>,
    },
}

impl RemoteRunHandle {
    fn is_confirmed(&self) -> bool {
        match self {
            Self::SshDirect {
                pgid, start_time, ..
            } => pgid.is_some() && start_time.is_some(),
            Self::LocalDetached {
                pgid,
                start_identity,
                ..
            } => {
                pgid.is_some()
                    && start_identity
                        .as_ref()
                        .is_some_and(|value| !value.is_empty())
            }
        }
    }

    fn inputs_staged(&self) -> bool {
        match self {
            Self::SshDirect { inputs_staged, .. } | Self::LocalDetached { inputs_staged, .. } => {
                *inputs_staged
            }
        }
    }

    fn mark_inputs_staged(&mut self) {
        match self {
            Self::SshDirect { inputs_staged, .. } | Self::LocalDetached { inputs_staged, .. } => {
                *inputs_staged = true
            }
        }
    }

    fn display_workdir(&self) -> String {
        match self {
            Self::SshDirect { workdir, .. }
            | Self::LocalDetached {
                transport: LocalTransport::Posix { .. },
                workdir,
                ..
            } => format!("~/{workdir}"),
            Self::LocalDetached {
                transport: LocalTransport::Windows { .. },
                workdir,
                ..
            } => format!("~\\{}", workdir.replace('/', "\\")),
        }
    }

    fn is_local_detached(&self) -> bool {
        matches!(self, Self::LocalDetached { .. })
    }
}

#[derive(Clone)]
struct RemoteRun {
    run_id: String,
    project_id: String,
    frame_id: Option<String>,
    command: String,
    timeout: Duration,
    input_refs: Vec<String>,
    output_specs: Vec<crate::harvest::OutputSpec>,
    harvest_root: Option<PathBuf>,
    handle: RemoteRunHandle,
}

#[derive(Clone)]
struct ActiveRun {
    abort: tokio::task::AbortHandle,
}

#[derive(Clone)]
pub struct RunManager {
    runner: Arc<dyn RunCommandRunner>,
    active: Arc<Mutex<HashMap<String, ActiveRun>>>,
    owner_id: String,
    reconciler_started: Arc<AtomicBool>,
    last_retention_sweep: Arc<Mutex<Option<Instant>>>,
}

const REMOTE_START_LEASE_SECS: i64 = 360;
const ACTIVE_LEASE_SECS: i64 = 30;
const RECONCILE_INTERVAL: Duration = Duration::from_secs(5);
const RETENTION_SWEEP_INTERVAL: Duration = Duration::from_secs(10 * 60);
const SSH_RETRY_STOPPED_MARKER: &str = "SSH automatic retry stopped";
const LOCAL_RETRY_STOPPED_MARKER: &str = "Automatic Run retry stopped";

impl RunManager {
    pub async fn has_in_flight_project(
        &self,
        store: &wisp_store::Store,
        project_id: &str,
    ) -> Result<bool, String> {
        let run_ids = self.active.lock().await.keys().cloned().collect::<Vec<_>>();
        for run_id in run_ids {
            if store
                .get_run(&run_id)
                .await
                .map_err(|error| error.to_string())?
                .is_some_and(|run| run.project_id == project_id)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn new() -> Self {
        Self::with_runner(Arc::new(ProcessRunRunner))
    }

    pub fn runner_ref(&self) -> &dyn RunCommandRunner {
        self.runner.as_ref()
    }

    pub fn with_runner(runner: Arc<dyn RunCommandRunner>) -> Self {
        Self {
            runner,
            active: Arc::new(Mutex::new(HashMap::new())),
            owner_id: uuid::Uuid::new_v4().to_string(),
            reconciler_started: Arc::new(AtomicBool::new(false)),
            last_retention_sweep: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn preflight(
        &self,
        store: &wisp_store::Store,
        context_id: &str,
        project_root: &Path,
        spec: &RunPreflightSpec,
    ) -> Result<RunPreflightReport, String> {
        let language = spec.language.trim().to_ascii_lowercase();
        let mut report = RunPreflightReport::new(context_id, &language);
        if !matches!(language.as_str(), "python" | "r") {
            report.push(
                "language",
                RunPreflightStatus::Failed,
                "language must be 'python' or 'r'".into(),
            );
            return Ok(report);
        }
        if spec.packages.len() > 32 || spec.paths.len() > 32 || spec.syntax_paths.len() > 32 {
            report.push(
                "limits",
                RunPreflightStatus::Failed,
                "preflight accepts at most 32 packages, paths, and syntax paths".into(),
            );
            return Ok(report);
        }

        let Some(context) = store
            .get_execution_context(context_id)
            .await
            .map_err(|error| error.to_string())?
        else {
            report.push(
                "context",
                RunPreflightStatus::Failed,
                format!("execution context not found: {context_id}"),
            );
            return Ok(report);
        };
        if context.kind == wisp_store::ExecutionContextKind::Ssh {
            if let Err(error) = crate::ssh_hosts::require_managed_ssh_ready(&context) {
                report.push("context", RunPreflightStatus::Failed, error);
                return Ok(report);
            }
        }
        match context.last_probe_status.as_deref() {
            Some("error") => report.push(
                "context",
                RunPreflightStatus::Warning,
                "the most recent context probe failed; the interpreter handshake will be retried"
                    .into(),
            ),
            _ => report.push(
                "context",
                RunPreflightStatus::Passed,
                format!(
                    "{} context is available for preflight",
                    context.kind.as_str()
                ),
            ),
        }

        let root = match std::fs::canonicalize(project_root) {
            Ok(root) => root,
            Err(error) => {
                report.push(
                    "working_directory",
                    RunPreflightStatus::Failed,
                    format!("cannot resolve the project working directory: {error}"),
                );
                return Ok(report);
            }
        };
        report.push(
            "working_directory",
            RunPreflightStatus::Passed,
            "project working directory resolved".into(),
        );

        let mut syntax_files = Vec::new();
        for path in spec.paths.iter().chain(spec.syntax_paths.iter()) {
            match resolve_preflight_path(&root, path) {
                Ok(resolved) => {
                    report.push(
                        format!("path:{path}"),
                        RunPreflightStatus::Passed,
                        "project-relative file exists".into(),
                    );
                    if spec.syntax_paths.contains(path) {
                        syntax_files.push(resolved);
                    }
                }
                Err(error) => {
                    report.push(format!("path:{path}"), RunPreflightStatus::Failed, error);
                }
            }
        }
        if report.status == RunPreflightStatus::Failed {
            return Ok(report);
        }

        if let Some(package) = spec
            .packages
            .iter()
            .find(|package| !valid_package_name(&language, package))
        {
            report.push(
                "packages",
                RunPreflightStatus::Failed,
                format!("invalid {language} package/module name: {package}"),
            );
            return Ok(report);
        }
        let interpreter = match preflight_interpreter(&context, &language) {
            Ok(interpreter) => interpreter,
            Err(error) => {
                report.push("interpreter", RunPreflightStatus::Failed, error);
                return Ok(report);
            }
        };
        let handshake = build_preflight_command(
            &context,
            &interpreter,
            preflight_handshake_args(&language, &spec.packages),
            Some(root.clone()),
            format!("{language} interpreter/package preflight"),
        )?;
        match self.runner.run(handshake, Duration::from_secs(5)).await {
            Ok(output) if output.exit_code == 0 => {
                let version = output.stdout.trim();
                report.push(
                    "interpreter",
                    RunPreflightStatus::Passed,
                    if version.is_empty() {
                        format!("{language} interpreter handshake passed")
                    } else {
                        format!(
                            "{language} interpreter handshake passed ({})",
                            tail(version)
                        )
                    },
                );
                if !spec.packages.is_empty() {
                    report.push(
                        "packages",
                        RunPreflightStatus::Passed,
                        format!("{} declared package(s) are available", spec.packages.len()),
                    );
                }
            }
            Ok(output) => {
                let detail = command_failure_detail(&output);
                report.push(
                    if spec.packages.is_empty() {
                        "interpreter"
                    } else {
                        "packages"
                    },
                    RunPreflightStatus::Failed,
                    format!(
                        "{language} preflight exited with code {}: {detail}",
                        output.exit_code
                    ),
                );
                return Ok(report);
            }
            Err(error) => {
                report.push(
                    "interpreter",
                    RunPreflightStatus::Failed,
                    format!("{language} interpreter preflight failed: {}", tail(&error)),
                );
                return Ok(report);
            }
        }

        if !syntax_files.is_empty() {
            if context.kind != wisp_store::ExecutionContextKind::Local {
                report.push(
                    "syntax",
                    RunPreflightStatus::Warning,
                    "syntax files are project-local and cannot be parsed in this remote context before staging"
                        .into(),
                );
            } else {
                let syntax = build_preflight_command(
                    &context,
                    &interpreter,
                    preflight_syntax_args(&language, &syntax_files),
                    Some(root),
                    format!("{language} syntax preflight"),
                )?;
                match self.runner.run(syntax, Duration::from_secs(5)).await {
                    Ok(output) if output.exit_code == 0 => report.push(
                        "syntax",
                        RunPreflightStatus::Passed,
                        format!("{} file(s) parsed successfully", syntax_files.len()),
                    ),
                    Ok(output) => report.push(
                        "syntax",
                        RunPreflightStatus::Failed,
                        format!(
                            "{language} syntax check exited with code {}: {}",
                            output.exit_code,
                            command_failure_detail(&output)
                        ),
                    ),
                    Err(error) => report.push(
                        "syntax",
                        RunPreflightStatus::Failed,
                        format!("{language} syntax check failed: {}", tail(&error)),
                    ),
                }
            }
        }
        Ok(report)
    }

    pub async fn download_ssh_file(
        &self,
        store: &wisp_store::Store,
        project_id: &str,
        frame_id: Option<&str>,
        context: &wisp_store::ExecutionContext,
        remote_path: &str,
        destination: &std::path::Path,
    ) -> Result<String, String> {
        if remote_path.is_empty() || remote_path.contains(['\0', '\n', '\r']) {
            return Err("Invalid remote file path".into());
        }
        remote_files::refuse_if_context_path_discarded(store, &context.id, remote_path).await?;
        crate::ssh_hosts::require_managed_ssh_ready(context)?;
        let connection = crate::ssh_hosts::SshConnection::from_execution_context(context)?;
        let size = remote_file_size(self.runner.as_ref(), &connection, remote_path).await?;
        let run_id = uuid::Uuid::new_v4().to_string();
        let file_name = std::path::Path::new(remote_path)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("download")
            .to_string();
        let started = Instant::now();
        let initial_progress = transfer_progress(
            "download",
            "downloading",
            0,
            size,
            0,
            1,
            Some(file_name.clone()),
            started,
        );
        let mut run = wisp_store::RunRecord::new(
            &run_id,
            project_id,
            &context.id,
            format!("Download {file_name}"),
            "file_transfer",
        );
        run.frame_id = frame_id.map(Into::into);
        run.command = Some(format!("download {remote_path}"));
        run.progress_json = serde_json::to_string(&initial_progress).map_err(|e| e.to_string())?;
        store.create_run(&run).await.map_err(|e| e.to_string())?;
        if !store
            .activate_run_lifecycle(
                &run_id,
                wisp_store::RunStatus::Running,
                &self.owner_id,
                ACTIVE_LEASE_SECS,
            )
            .await
            .map_err(|e| e.to_string())?
        {
            return Err("Download Run changed state before it could start".into());
        }
        let mut args = connection.scp_option_args()?;
        args.push(format!("{}:{remote_path}", connection.target()?));
        args.push(destination.to_string_lossy().into_owned());
        let command = RunCommand {
            context_id: context.id.clone(),
            program: "scp".into(),
            args,
            script: format!("download {remote_path}"),
            cwd: destination.parent().map(std::path::Path::to_path_buf),
            stdin: None,
            envs: crate::ssh_hosts::auth_envs_for_connection(&connection)?,
        };
        let runner = self.runner.clone();
        let task_store = store.clone();
        let owner_id = self.owner_id.clone();
        let task_run_id = run_id.clone();
        let destination = destination.to_path_buf();
        let task = tokio::spawn(async move {
            download_lifecycle(
                &task_store,
                &owner_id,
                &task_run_id,
                runner.as_ref(),
                command,
                &destination,
                size,
                file_name,
                started,
            )
            .await
        });
        let abort = task.abort_handle();
        self.active
            .lock()
            .await
            .insert(run_id.clone(), ActiveRun { abort });
        let result = task.await;
        self.active.lock().await.remove(&run_id);
        match result {
            Ok(result) => result.map(|_| run_id),
            Err(error) if error.is_cancelled() => Err("download cancelled".into()),
            Err(error) => Err(format!("download task failed: {error}")),
        }
    }

    pub async fn recover(&self, store: &wisp_store::Store) -> Result<u64, String> {
        self.start_reconciler(store.clone());
        self.reconcile_once(store).await
    }

    fn start_reconciler(&self, store: wisp_store::Store) {
        if self.reconciler_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let manager = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(RECONCILE_INTERVAL).await;
                if let Err(error) = manager.reconcile_once(&store).await {
                    tracing::warn!("Run lifecycle reconciliation failed: {error}");
                }
            }
        });
    }

    /// Opt-in automatic reclamation of expired run workspaces. Every cleanup
    /// goes through `cleanup_run_workspace`, so all of its preconditions and
    /// path constraints apply; one failing run never blocks the sweep.
    pub async fn run_retention_sweep(&self, store: &wisp_store::Store) -> Result<u64, String> {
        let now = chrono::Utc::now().timestamp();
        let due = store
            .list_runs_due_for_retention(now)
            .await
            .map_err(|e| e.to_string())?;
        let mut cleaned = 0;
        for run in due {
            match self.cleanup_run_workspace(store, &run.id, false).await {
                Ok(_) => cleaned += 1,
                Err(error) => {
                    tracing::warn!(run_id = %run.id, "retention cleanup failed: {error}");
                }
            }
        }
        Ok(cleaned)
    }

    /// Opt-in automatic reclamation of orphaned ledgered remote files: staged
    /// uploads and persisted outputs that nothing references anymore, past
    /// the project's orphan window. Replaced ledger entries are closed
    /// in-ledger only — the newer entry owns those remote bytes. Only
    /// ledgered paths are ever deleted, through the same safe channel as the
    /// manual tool.
    pub async fn orphan_file_sweep(&self, store: &wisp_store::Store) -> Result<u64, String> {
        let now = chrono::Utc::now().timestamp();
        let candidates = store
            .list_orphan_gc_contexts(now)
            .await
            .map_err(|e| e.to_string())?;
        let mut removed = 0;
        for (project_id, context_id, cutoff) in candidates {
            let Some(context) = store
                .get_execution_context(&context_id)
                .await
                .map_err(|e| e.to_string())?
            else {
                continue;
            };
            let views = match remote_files::list_remote_files(store, &project_id, &context_id).await
            {
                Ok(views) => views,
                Err(error) => {
                    tracing::warn!(context_id, "orphan sweep listing failed: {error}");
                    continue;
                }
            };
            let due = |view: &remote_files::RemoteFileView| view.created_at < cutoff;
            let replaced: Vec<String> = views
                .iter()
                .filter(|view| view.state == remote_files::RemoteFileState::Replaced && due(view))
                .map(|view| view.id.clone())
                .collect();
            if !replaced.is_empty() {
                removed += store
                    .mark_remote_staging_removed(&replaced)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            let orphans: Vec<String> = views
                .iter()
                .filter(|view| view.state == remote_files::RemoteFileState::Orphan && due(view))
                .map(|view| view.id.clone())
                .collect();
            if orphans.is_empty() {
                continue;
            }
            match remote_files::remove_remote_files(
                store,
                self.runner.as_ref(),
                &project_id,
                &context,
                &orphans,
                false,
            )
            .await
            {
                Ok(count) => removed += count,
                Err(error) => {
                    tracing::warn!(context_id, "orphan sweep deletion failed: {error}");
                }
            }
        }
        Ok(removed)
    }

    async fn maybe_run_retention_sweep(&self, store: &wisp_store::Store) {
        {
            let mut last = self.last_retention_sweep.lock().await;
            let due = last
                .map(|at| at.elapsed() >= RETENTION_SWEEP_INTERVAL)
                .unwrap_or(true);
            if !due {
                return;
            }
            *last = Some(Instant::now());
        }
        if let Err(error) = self.run_retention_sweep(store).await {
            tracing::warn!("run retention sweep failed: {error}");
        }
        if let Err(error) = self.orphan_file_sweep(store).await {
            tracing::warn!("orphan file sweep failed: {error}");
        }
    }

    async fn reconcile_once(&self, store: &wisp_store::Store) -> Result<u64, String> {
        self.maybe_run_retention_sweep(store).await;
        let runs = store.list_active_runs().await.map_err(|e| e.to_string())?;
        let mut lost = 0;
        for run in runs {
            if self.active.lock().await.contains_key(&run.id) {
                continue;
            }
            let lease_secs = run
                .remote_handle_json
                .as_deref()
                .and_then(|json| serde_json::from_str::<RemoteRunHandle>(json).ok())
                .map(|handle| {
                    if handle.is_confirmed() {
                        ACTIVE_LEASE_SECS
                    } else {
                        REMOTE_START_LEASE_SECS
                    }
                })
                .unwrap_or(ACTIVE_LEASE_SECS);
            let claimed = store
                .claim_run_lifecycle(&run.id, &self.owner_id, lease_secs)
                .await
                .map_err(|e| e.to_string())?;
            if !claimed {
                continue;
            }
            if run.kind == "file_transfer" {
                if let Err(error) = self.reclaim_transfer(store.clone(), &run).await {
                    tracing::warn!(run_id = %run.id, "transfer reclaim failed: {error}");
                }
                continue;
            }
            match remote_run_from_record(store, &run).await {
                Ok(Some(remote)) => self.spawn_remote_claimed(store.clone(), remote).await,
                Ok(None) => {
                    if store
                        .mark_run_lost_owned(&run.id, &self.owner_id)
                        .await
                        .map_err(|e| e.to_string())?
                    {
                        lost += 1;
                    }
                }
                Err(error) => {
                    let _ = store
                        .record_run_poll_owned(&run.id, &self.owner_id, None, None, Some(&error))
                        .await
                        .map_err(|e| e.to_string())?;
                    if store
                        .mark_run_lost_owned(&run.id, &self.owner_id)
                        .await
                        .map_err(|e| e.to_string())?
                    {
                        lost += 1;
                    }
                }
            }
        }
        Ok(lost)
    }

    pub async fn submit(
        &self,
        store: wisp_store::Store,
        project_id: String,
        frame_id: Option<String>,
        request: SubmitRunRequest,
        cwd: Option<PathBuf>,
    ) -> Result<SubmitRunResponse, String> {
        self.submit_inner(store, project_id, frame_id, request, cwd, None)
            .await
    }

    pub async fn submit_preflighted(
        &self,
        store: wisp_store::Store,
        project_id: String,
        frame_id: Option<String>,
        request: SubmitRunRequest,
        cwd: Option<PathBuf>,
        preflight: RunPreflightReport,
    ) -> Result<SubmitRunResponse, String> {
        self.submit_inner(store, project_id, frame_id, request, cwd, Some(preflight))
            .await
    }

    async fn submit_inner(
        &self,
        store: wisp_store::Store,
        project_id: String,
        frame_id: Option<String>,
        request: SubmitRunRequest,
        cwd: Option<PathBuf>,
        preflight: Option<RunPreflightReport>,
    ) -> Result<SubmitRunResponse, String> {
        let prepared = create_run_record(
            &store,
            &project_id,
            frame_id.as_deref(),
            request,
            cwd,
            wisp_store::RunStatus::Submitted,
            &self.owner_id,
            REMOTE_START_LEASE_SECS,
            preflight.as_ref(),
        )
        .await?;
        if let Some(remote) = prepared.remote.clone() {
            self.spawn_remote_claimed(store.clone(), remote).await;
            let run = store
                .get_run(&prepared.run_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "Run disappeared after SSH submission".to_string())?;
            return Ok(response_from_run(&run));
        }

        let run_id = prepared.run_id.clone();
        let task_store = store.clone();
        let runner = self.runner.clone();
        let active = self.active.clone();
        let cleanup_id = run_id.clone();
        let task_run_id = cleanup_id.clone();
        let task = tokio::spawn(async move {
            let result: Result<(), String> = async {
                if !task_store
                    .transition_run_to_running_owned(&prepared.run_id, &prepared.owner_id)
                    .await
                    .map_err(|e| e.to_string())?
                {
                    return Ok(());
                }
                let output = run_with_lifecycle_lease(
                    &task_store,
                    &prepared.run_id,
                    &prepared.owner_id,
                    runner.as_ref(),
                    prepared.command.clone(),
                    prepared.timeout,
                )
                .await;
                record_run_outcome(&task_store, &prepared, output, &prepared.owner_id).await?;
                Ok(())
            }
            .await;
            if let Err(error) = result {
                tracing::warn!(run_id = %task_run_id, "background run failed: {error}");
            }
        });
        let handle = task.abort_handle();
        self.active
            .lock()
            .await
            .insert(run_id.clone(), ActiveRun { abort: handle });
        tokio::spawn(async move {
            let _ = task.await;
            active.lock().await.remove(&cleanup_id);
        });
        Ok(SubmitRunResponse {
            run_id,
            status: wisp_store::RunStatus::Submitted,
            exit_code: None,
            stdout_tail: None,
            stderr_tail: None,
            remote_workdir: None,
        })
    }

    async fn spawn_remote(
        &self,
        store: wisp_store::Store,
        remote: RemoteRun,
    ) -> Result<bool, String> {
        if self.active.lock().await.contains_key(&remote.run_id) {
            return Ok(false);
        }
        let lease_secs = remote_lifecycle_lease_secs(&remote);
        let claimed = store
            .claim_run_lifecycle(&remote.run_id, &self.owner_id, lease_secs)
            .await
            .map_err(|e| e.to_string())?;
        if !claimed {
            return Ok(false);
        }
        self.spawn_remote_claimed(store, remote).await;
        Ok(true)
    }

    async fn spawn_remote_claimed(&self, store: wisp_store::Store, remote: RemoteRun) {
        let run_id = remote.run_id.clone();
        let mut active_runs = self.active.lock().await;
        if active_runs.contains_key(&run_id) {
            return;
        }
        let runner = self.runner.clone();
        let active = self.active.clone();
        let owner_id = self.owner_id.clone();
        let cleanup_id = run_id.clone();
        let task_run_id = run_id.clone();
        let task = tokio::spawn(async move {
            loop {
                match remote_lifecycle(&store, runner.as_ref(), &owner_id, remote.clone()).await {
                    Ok(()) => break,
                    Err(error) => {
                        tracing::warn!(run_id = %task_run_id, "SSH run lifecycle failed: {error}");
                        tokio::time::sleep(remote_poll_interval(1)).await;
                        match store.get_run(&task_run_id).await {
                            Ok(Some(run)) if !run.status.is_terminal() => {}
                            Ok(_) => break,
                            Err(_) => {}
                        }
                    }
                }
            }
        });
        let abort = task.abort_handle();
        active_runs.insert(run_id, ActiveRun { abort });
        drop(active_runs);
        tokio::spawn(async move {
            let _ = task.await;
            active.lock().await.remove(&cleanup_id);
        });
    }

    /// Cancel every in-flight Run for a project, then best-effort clean their
    /// server workspaces. Called before `delete_project` so dropping the SQLite
    /// rows does not abandon live nohup jobs.
    pub async fn wind_down_project(
        &self,
        store: &wisp_store::Store,
        project_id: &str,
    ) -> Result<(), String> {
        let active = store
            .list_active_runs_for_project(project_id)
            .await
            .map_err(|e| e.to_string())?;
        for run in &active {
            if let Err(error) = self.cancel(store, &run.id).await {
                tracing::warn!(run_id = %run.id, "wind-down cancel failed: {error}");
            }
        }
        let runs = store
            .list_uncleaned_runs_for_project(project_id)
            .await
            .map_err(|e| e.to_string())?;
        for run in runs {
            if run.kind == "file_transfer" {
                continue;
            }
            if let Err(error) = self.cleanup_run_workspace(store, &run.id, true).await {
                tracing::warn!(run_id = %run.id, "wind-down cleanup failed: {error}");
            }
        }
        Ok(())
    }

    /// Cancel in-flight Runs on one SSH context across every project.
    /// Called before dropping a host so nohup jobs do not keep running after
    /// Wisp forgets the machine. Remote bytes are abandoned, not deleted.
    pub async fn wind_down_context(
        &self,
        store: &wisp_store::Store,
        context_id: &str,
    ) -> Result<(), String> {
        let active = store
            .list_active_runs_for_context(context_id)
            .await
            .map_err(|e| e.to_string())?;
        for run in &active {
            if let Err(error) = self.cancel(store, &run.id).await {
                tracing::warn!(run_id = %run.id, "context wind-down cancel failed: {error}");
            }
        }
        Ok(())
    }

    /// Delete a terminal Run's server-side workspace. `force` is an explicit
    /// user confirmation that skips the harvested-before-clean guard (the
    /// agent/automatic paths never pass it).
    pub async fn cleanup_run_workspace(
        &self,
        store: &wisp_store::Store,
        run_id: &str,
        force: bool,
    ) -> Result<wisp_store::RunRecord, String> {
        let run = store
            .get_run(run_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Run not found: {run_id}"))?;
        if !run.status.is_terminal() {
            return Err(format!(
                "Run is still {}; cancel or wait for it before cleaning its workspace",
                run.status.as_str()
            ));
        }
        if run.cleaned_at.is_some() {
            return Ok(run);
        }
        let output_specs: Vec<crate::harvest::OutputSpec> =
            serde_json::from_str(&run.output_specs_json).unwrap_or_default();
        if run.status == wisp_store::RunStatus::Succeeded
            && !output_specs.is_empty()
            && run.harvested_at.is_none()
            && !force
        {
            return Err(
                "Run outputs were never harvested; call harvest_run first, or clean up with \
                 explicit user confirmation (force) accepting that unretrieved outputs are lost"
                    .into(),
            );
        }
        let handle: RemoteRunHandle = run
            .remote_handle_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
            .ok_or_else(|| "Run has no server workspace to clean".to_string())?;
        // Defensive: never delete a workdir that a registered External
        // artifact reference still points into.
        let workdir_fragment = match &handle {
            RemoteRunHandle::SshDirect { workdir, .. }
            | RemoteRunHandle::LocalDetached { workdir, .. } => format!("/{workdir}/"),
        };
        for output in store
            .list_run_outputs(run_id)
            .await
            .map_err(|e| e.to_string())?
        {
            let Some(version) = store
                .get_artifact_version(&output.artifact_version_id)
                .await
                .map_err(|e| e.to_string())?
            else {
                continue;
            };
            if version.materialization == wisp_store::ArtifactMaterialization::External
                && version.storage_path.contains(&workdir_fragment)
            {
                return Err(format!(
                    "Registered artifact reference {} still points into the run workspace; \
                     harvest it before cleanup",
                    version.storage_path
                ));
            }
        }
        // Never destroy the only copy of the run's logs: pull them into the
        // project first. A failed pull aborts cleanup and stays retryable.
        if run.logs_path.is_none() {
            match save_run_logs_locally(store, self.runner.as_ref(), &run, &handle).await {
                Ok(Some(relative)) => {
                    let _ = store
                        .mark_run_logs_saved(run_id, &relative)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                Ok(None) => {}
                Err(error) => {
                    let error = format!("saving run logs before cleanup failed: {error}");
                    let _ = store.record_run_cleanup_error(run_id, &error).await;
                    return Err(error);
                }
            }
        }
        match cleanup::delete_run_workspace(self.runner.as_ref(), &handle, run_id).await {
            Ok(()) => {
                store
                    .mark_run_cleaned(run_id)
                    .await
                    .map_err(|e| e.to_string())?;
                // The workdir took its staged inputs with it.
                let _ = store.mark_remote_staging_removed_for_run(run_id).await;
                store
                    .get_run(run_id)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "Run disappeared after cleanup".to_string())
            }
            Err(error) => {
                let _ = store.record_run_cleanup_error(run_id, &error).await;
                Err(error)
            }
        }
    }

    async fn terminal_ssh_remote(
        &self,
        store: &wisp_store::Store,
        run_id: &str,
    ) -> Result<RemoteRun, String> {
        let run = store
            .get_run(run_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Run not found: {run_id}"))?;
        if !run.status.is_terminal() {
            return Err(format!(
                "Run is still {}; wait for it to finish first",
                run.status.as_str()
            ));
        }
        if run.cleaned_at.is_some() {
            return Err("Run workspace was already cleaned".into());
        }
        let remote = remote_run_from_record(store, &run)
            .await?
            .ok_or_else(|| "Run has no server workspace".to_string())?;
        if remote.handle.is_local_detached() {
            return Err("This Run executed locally; its files are already in the project".into());
        }
        Ok(remote)
    }

    /// Whether the results-review modal is worth auto-opening for this Run.
    /// The prompt is reserved for submitted-task Runs with an unresolved
    /// product decision: declared outputs that were never harvested (data at
    /// risk on the server), or no declared outputs but files present in the
    /// workspace. Exploratory command Runs, harvested Runs, cleaned Runs, and
    /// Runs whose prompt the user already closed all return false.
    pub async fn should_prompt_run_review(
        &self,
        store: &wisp_store::Store,
        run_id: &str,
    ) -> Result<bool, String> {
        let run = store
            .get_run(run_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Run not found: {run_id}"))?;
        if run.kind != "ssh_direct"
            || run.status != wisp_store::RunStatus::Succeeded
            || run.cleaned_at.is_some()
            || run.remote_workdir.is_none()
        {
            return Ok(false);
        }
        if store
            .run_review_dismissed(run_id)
            .await
            .map_err(|e| e.to_string())?
        {
            return Ok(false);
        }
        let has_output_specs = !matches!(run.output_specs_json.trim(), "" | "[]");
        if has_output_specs {
            // Harvest already registered the declared products locally; the
            // remaining cleanup decision is not worth an interruption. An
            // unharvested Run means results only exist on the server.
            return Ok(run.harvested_at.is_none());
        }
        // No declared products: interrupt only when the workspace actually
        // holds files to review. Listing errors (host unreachable) suppress
        // the prompt rather than surfacing a modal that cannot browse.
        let listing = self
            .list_run_workspace_files(store, run_id, "", "", 0, 1)
            .await?;
        Ok(!listing.entries.is_empty())
    }

    /// One page of one directory level of a finished Run's server workspace.
    /// Ephemeral data — nothing here is persisted.
    pub async fn list_run_workspace_files(
        &self,
        store: &wisp_store::Store,
        run_id: &str,
        path: &str,
        name_filter: &str,
        offset: usize,
        limit: usize,
    ) -> Result<harvest_remote::WorkspaceListing, String> {
        let remote = self.terminal_ssh_remote(store, run_id).await?;
        harvest_remote::list_run_workspace_files(
            self.runner.as_ref(),
            &remote,
            path,
            name_filter,
            offset,
            limit,
        )
        .await
    }

    /// Download the user's explicit selection from a finished Run's workspace
    /// and register it (files individually, directories as one archive each).
    pub async fn download_run_files(
        &self,
        store: &wisp_store::Store,
        run_id: &str,
        files: &[String],
        dirs: &[String],
    ) -> Result<Vec<crate::harvest::HarvestedArtifact>, String> {
        for path in files.iter().chain(dirs) {
            harvest_remote::validate_workspace_subpath(path)?;
        }
        let remote = self.terminal_ssh_remote(store, run_id).await?;
        if remote.frame_id.is_none() {
            return Err("Run has no source session to register artifacts under".into());
        }
        harvest_remote::download_run_files(
            store,
            self.runner.as_ref(),
            &self.owner_id,
            &remote,
            files,
            dirs,
        )
        .await
    }

    /// Delete selected files/directories inside a finished Run's workspace.
    /// User-explicit by construction (the review modal is the only caller), so
    /// no harvest guard applies; whole-workspace deletion still goes through
    /// `cleanup_run_workspace`.
    pub async fn delete_run_files(
        &self,
        store: &wisp_store::Store,
        run_id: &str,
        paths: &[String],
    ) -> Result<(), String> {
        let remote = self.terminal_ssh_remote(store, run_id).await?;
        harvest_remote::delete_run_files(self.runner.as_ref(), &remote, paths).await
    }

    /// Retry output harvest for a succeeded Run whose outputs were never
    /// registered (automatic harvest failed or the app closed mid-pull).
    pub async fn harvest_run(
        &self,
        store: &wisp_store::Store,
        run_id: &str,
    ) -> Result<Vec<crate::harvest::HarvestedArtifact>, String> {
        let run = store
            .get_run(run_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Run not found: {run_id}"))?;
        if run.status != wisp_store::RunStatus::Succeeded {
            return Err(format!(
                "harvest_run requires a succeeded Run, but it is {}",
                run.status.as_str()
            ));
        }
        if run.harvested_at.is_some() {
            return Err("Run outputs were already harvested".into());
        }
        let remote = remote_run_from_record(store, &run)
            .await?
            .ok_or_else(|| "Run has no detached handle to harvest from".to_string())?;
        if remote.output_specs.is_empty() {
            return Err("Run declared no output specs".into());
        }
        let frame_id = remote
            .frame_id
            .clone()
            .ok_or_else(|| "Run has no source session".to_string())?;
        let result = if remote.handle.is_local_detached() {
            let root = remote
                .harvest_root
                .clone()
                .ok_or_else(|| "Run project workspace is not available".to_string())?;
            let references: Vec<_> = remote
                .output_specs
                .iter()
                .filter(|spec| !spec.glob.starts_with("ssh://"))
                .cloned()
                .collect();
            crate::harvest::harvest_run_outputs(
                store,
                &remote.project_id,
                &frame_id,
                &remote.run_id,
                &root,
                &references,
            )
            .await
        } else {
            let fallback = PathBuf::from(".");
            let uri_references: Vec<_> = remote
                .output_specs
                .iter()
                .filter(|spec| spec.glob.starts_with("ssh://"))
                .cloned()
                .collect();
            if !uri_references.is_empty() {
                crate::harvest::harvest_run_outputs(
                    store,
                    &remote.project_id,
                    &frame_id,
                    &remote.run_id,
                    remote.harvest_root.as_deref().unwrap_or(&fallback),
                    &uri_references,
                )
                .await?;
            }
            harvest_remote::harvest_ssh_run(
                store,
                self.runner.as_ref(),
                &self.owner_id,
                &remote,
                false,
            )
            .await
        };
        match result {
            Ok(harvested) => {
                store
                    .mark_run_harvested(run_id)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(harvested)
            }
            Err(error) => {
                let _ = store
                    .record_run_harvest_error(
                        run_id,
                        &format!("remote artifact registration failed: {error}"),
                    )
                    .await;
                Err(error)
            }
        }
    }

    pub async fn cancel(&self, store: &wisp_store::Store, run_id: &str) -> Result<(), String> {
        let run = store
            .get_run(run_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Run not found: {run_id}"))?;
        if run.status.is_terminal() {
            return Err(format!("Run is already {}", run.status.as_str()));
        }
        let already_cancelling = run.status == wisp_store::RunStatus::Cancelling;
        let requested = if already_cancelling {
            false
        } else {
            store
                .request_run_cancellation(run_id)
                .await
                .map_err(|e| e.to_string())?
        };
        let refreshed = store
            .get_run(run_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Run not found: {run_id}"))?;
        if refreshed.status.is_terminal() {
            // The lifecycle task can confirm the cancellation requested above
            // before this re-read; that is success, not an error.
            if requested && refreshed.status == wisp_store::RunStatus::Cancelled {
                return Ok(());
            }
            return Err(format!("Run is already {}", refreshed.status.as_str()));
        }
        if refreshed.kind == "file_transfer" {
            if let Some(active) = self.active.lock().await.remove(run_id) {
                active.abort.abort();
            }
            if already_cancelling {
                mark_transfer_progress_cancelled(store, &self.owner_id, &refreshed).await;
                let _ = store
                    .force_finish_cancelling_run(run_id, wisp_store::RunStatus::Cancelled, None)
                    .await
                    .map_err(|e| e.to_string())?;
            } else if requested {
                mark_transfer_progress_cancelled(store, &self.owner_id, &refreshed).await;
                let _ = store
                    .finish_active_run_owned(
                        run_id,
                        &self.owner_id,
                        wisp_store::RunStatus::Cancelled,
                        None,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
            }
            return Ok(());
        }
        if let Some(remote) = remote_run_from_record(store, &refreshed).await? {
            // A second cancel while already Cancelling force-finishes the Run so
            // a wedged cancel/poll RPC cannot leave the UI stuck forever.
            if already_cancelling {
                if let Some(active) = self.active.lock().await.remove(run_id) {
                    active.abort.abort();
                }
                let finished = store
                    .force_finish_cancelling_run(run_id, wisp_store::RunStatus::Cancelled, None)
                    .await
                    .map_err(|e| e.to_string())?;
                if !finished {
                    return Err("Run is no longer cancelling".into());
                }
                return Ok(());
            }
            let uploading =
                serde_json::from_str::<wisp_store::RunProgress>(&refreshed.progress_json)
                    .is_ok_and(|progress| progress.phase == "uploading");
            if requested && uploading && !remote.handle.is_confirmed() {
                if let Some(active) = self.active.lock().await.remove(run_id) {
                    active.abort.abort();
                }
                mark_transfer_progress_cancelled(store, &self.owner_id, &refreshed).await;
                let _ = store
                    .finish_active_run_owned(
                        run_id,
                        &self.owner_id,
                        wisp_store::RunStatus::Cancelled,
                        None,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                return Ok(());
            }
            if !self.active.lock().await.contains_key(run_id) {
                let _ = self.spawn_remote(store.clone(), remote).await?;
            }
            return Ok(());
        }
        if refreshed.context_id.starts_with("ssh:") {
            return Err("SSH Run is missing its persisted remote handle".into());
        }
        if let Some(active) = self.active.lock().await.remove(run_id) {
            active.abort.abort();
        }
        if already_cancelling {
            let _ = store
                .force_finish_cancelling_run(run_id, wisp_store::RunStatus::Cancelled, None)
                .await
                .map_err(|e| e.to_string())?;
        } else if requested {
            let _ = store
                .finish_active_run_owned(
                    run_id,
                    &self.owner_id,
                    wisp_store::RunStatus::Cancelled,
                    None,
                )
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

async fn mark_transfer_progress_cancelled(
    store: &wisp_store::Store,
    owner_id: &str,
    run: &wisp_store::RunRecord,
) {
    let Ok(mut progress) = serde_json::from_str::<wisp_store::RunProgress>(&run.progress_json)
    else {
        return;
    };
    progress.phase = "cancelled".into();
    progress.current_file = None;
    progress.bytes_per_second = None;
    progress.eta_seconds = None;
    progress.updated_at = chrono::Utc::now().timestamp();
    let _ = store
        .update_run_progress_owned(&run.id, owner_id, &progress)
        .await;
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn remote_path_assignment(path: &str) -> String {
    match path {
        "~" => "path=\"$HOME\"".into(),
        _ if path.starts_with("~/") => {
            format!("path=\"$HOME\"/{}", shell_single_quote(&path[2..]))
        }
        _ => format!("path={}", shell_single_quote(path)),
    }
}

async fn remote_file_size(
    runner: &dyn RunCommandRunner,
    connection: &crate::ssh_hosts::SshConnection,
    remote_path: &str,
) -> Result<u64, String> {
    let payload = format!(
        "set -eu\n{}\n[ -f \"$path\" ] || {{ echo 'remote file not found' >&2; exit 66; }}\nbytes=$(wc -c < \"$path\")\nprintf '__WISP_TRANSFER_SIZE__:%s\\n' \"$bytes\"\n",
        remote_path_assignment(remote_path)
    );
    let output = checked_output(
        "SSH download size",
        runner
            .run(
                ssh_script_command(connection, "measure SSH download", payload)?,
                REMOTE_RPC_TIMEOUT,
            )
            .await,
    )?;
    output
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix("__WISP_TRANSFER_SIZE__:"))
        .ok_or_else(|| "SSH download size response was missing".to_string())?
        .trim()
        .parse::<u64>()
        .map_err(|_| "SSH download size response was invalid".to_string())
}

async fn download_lifecycle(
    store: &wisp_store::Store,
    owner_id: &str,
    run_id: &str,
    runner: &dyn RunCommandRunner,
    command: RunCommand,
    destination: &std::path::Path,
    total_bytes: u64,
    file_name: String,
    started: Instant,
) -> Result<(), String> {
    let transfer = runner.run(command, Duration::from_secs(4 * 60 * 60));
    tokio::pin!(transfer);
    let mut interval = tokio::time::interval(if cfg!(test) {
        Duration::from_millis(10)
    } else {
        Duration::from_secs(1)
    });
    interval.tick().await;
    let output = loop {
        tokio::select! {
            output = &mut transfer => break output,
            _ = interval.tick() => {
                if !store.renew_run_lifecycle(run_id, owner_id, ACTIVE_LEASE_SECS)
                    .await.map_err(|error| error.to_string())? {
                    return Err("Download lifecycle lease expired".into());
                }
                let completed = tokio::fs::metadata(destination)
                    .await.map(|metadata| metadata.len()).unwrap_or(0);
                let progress = transfer_progress(
                    "download", "downloading", completed, total_bytes, 0, 1,
                    Some(file_name.clone()), started,
                );
                if !store.update_run_progress_owned(run_id, owner_id, &progress)
                    .await.map_err(|error| error.to_string())? {
                    return Err("Download lifecycle lease expired".into());
                }
            }
        }
    };
    let (status, exit_code, stdout, stderr, result) = match output {
        Ok(output) if output.exit_code == 0 => (
            wisp_store::RunStatus::Succeeded,
            Some(0),
            output.stdout,
            output.stderr,
            Ok(()),
        ),
        Ok(output) => {
            let detail = if output.stderr.trim().is_empty() {
                output.stdout.trim().to_string()
            } else {
                output.stderr.trim().to_string()
            };
            let error = format!("scp download failed (exit {}): {detail}", output.exit_code);
            (
                wisp_store::RunStatus::Failed,
                Some(output.exit_code),
                output.stdout,
                output.stderr,
                Err(error),
            )
        }
        Err(error) => (
            wisp_store::RunStatus::Failed,
            Some(-1),
            String::new(),
            error.clone(),
            Err(error),
        ),
    };
    let completed = if status == wisp_store::RunStatus::Succeeded {
        total_bytes
    } else {
        tokio::fs::metadata(destination)
            .await
            .map(|metadata| metadata.len())
            .unwrap_or(0)
    };
    let progress = transfer_progress(
        "download",
        if status == wisp_store::RunStatus::Succeeded {
            "downloaded"
        } else {
            "failed"
        },
        completed,
        total_bytes,
        u64::from(status == wisp_store::RunStatus::Succeeded),
        1,
        None,
        started,
    );
    let _ = store
        .renew_run_lifecycle(run_id, owner_id, ACTIVE_LEASE_SECS)
        .await;
    let _ = store
        .update_run_progress_owned(run_id, owner_id, &progress)
        .await;
    let _ = store
        .update_run_output_owned(run_id, owner_id, Some(&tail(&stdout)), Some(&tail(&stderr)))
        .await;
    store
        .finish_active_run_owned(run_id, owner_id, status, exit_code)
        .await
        .map_err(|error| error.to_string())?;
    result
}

impl Default for RunManager {
    fn default() -> Self {
        Self::new()
    }
}

fn resolve_preflight_path(root: &Path, value: &str) -> Result<PathBuf, String> {
    let relative = Path::new(value);
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("preflight path must be project-relative: {value}"));
    }
    let path = std::fs::canonicalize(root.join(relative))
        .map_err(|error| format!("preflight file does not exist ({value}): {error}"))?;
    if !path.starts_with(root) || !path.is_file() {
        return Err(format!("preflight path is not a project file: {value}"));
    }
    Ok(path)
}

fn valid_package_name(language: &str, value: &str) -> bool {
    if value.is_empty() || value.len() > 128 {
        return false;
    }
    value.split('.').all(|part| {
        let mut chars = part.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        let valid_first = if language == "python" {
            first == '_' || first.is_ascii_alphabetic()
        } else {
            first.is_ascii_alphabetic()
        };
        valid_first
            && chars
                .all(|ch| ch == '_' || ch.is_ascii_alphanumeric() || language == "r" && ch == '-')
    })
}

fn context_json_string(context: &wisp_store::ExecutionContext, keys: &[&str]) -> Option<String> {
    [&context.config_json, &context.capabilities_json]
        .into_iter()
        .filter_map(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .find_map(|value| {
            keys.iter().find_map(|key| {
                value
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            })
        })
}

fn preflight_interpreter(
    context: &wisp_store::ExecutionContext,
    language: &str,
) -> Result<String, String> {
    let (keys, fallback) = match language {
        "python" => (
            &["python_executable", "python_path"][..],
            if cfg!(target_os = "windows") {
                "python"
            } else {
                "python3"
            },
        ),
        "r" => (&["rscript_executable", "rscript_path"][..], "Rscript"),
        _ => return Err(format!("unsupported preflight language: {language}")),
    };
    Ok(context_json_string(context, keys).unwrap_or_else(|| fallback.into()))
}

fn preflight_handshake_args(language: &str, packages: &[String]) -> Vec<String> {
    if language == "python" {
        let packages = serde_json::to_string(packages).unwrap_or_else(|_| "[]".into());
        let code = format!(
            "import importlib.util,sys; p={packages}; m=[x for x in p if importlib.util.find_spec(x) is None]; print(sys.version.split()[0]); sys.stderr.write('missing modules: '+', '.join(m) if m else ''); sys.exit(9 if m else 0)"
        );
        vec!["-c".into(), code]
    } else {
        let packages = packages
            .iter()
            .map(|package| format!("\"{package}\""))
            .collect::<Vec<_>>()
            .join(",");
        let code = format!(
            "p <- c({packages}); m <- p[!vapply(p, requireNamespace, logical(1), quietly=TRUE)]; cat(R.version.string); if (length(m)) {{ cat(paste0('missing packages: ', paste(m, collapse=', ')), file=stderr()); quit(status=9) }}"
        );
        vec!["--vanilla".into(), "-e".into(), code]
    }
}

fn preflight_syntax_args(language: &str, paths: &[PathBuf]) -> Vec<String> {
    if language == "python" {
        let mut args = vec!["-m".into(), "py_compile".into()];
        args.extend(paths.iter().map(|path| path.to_string_lossy().into_owned()));
        args
    } else {
        let mut args = vec![
            "--vanilla".into(),
            "-e".into(),
            "invisible(lapply(commandArgs(TRUE), function(path) parse(file=path)))".into(),
            "--args".into(),
        ];
        args.extend(paths.iter().map(|path| path.to_string_lossy().into_owned()));
        args
    }
}

fn posix_shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn build_preflight_command(
    context: &wisp_store::ExecutionContext,
    interpreter: &str,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    label: String,
) -> Result<RunCommand, String> {
    match context.kind {
        wisp_store::ExecutionContextKind::Local => Ok(RunCommand {
            context_id: context.id.clone(),
            program: interpreter.into(),
            args,
            script: label,
            cwd,
            stdin: None,
            envs: Vec::new(),
        }),
        wisp_store::ExecutionContextKind::Wsl => {
            let config: serde_json::Value =
                serde_json::from_str(&context.config_json).unwrap_or_default();
            let distro = config
                .get("distro")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_else(|| context.id.strip_prefix("wsl:").unwrap_or(&context.id));
            let mut command_args =
                vec!["-d".into(), distro.into(), "--".into(), interpreter.into()];
            command_args.extend(args);
            Ok(RunCommand {
                context_id: context.id.clone(),
                program: "wsl.exe".into(),
                args: command_args,
                script: label,
                cwd: None,
                stdin: None,
                envs: Vec::new(),
            })
        }
        wisp_store::ExecutionContextKind::Ssh => {
            let connection = crate::ssh_hosts::SshConnection::from_execution_context(context)?;
            let mut ssh_args = connection.ssh_args()?;
            let command = std::iter::once(interpreter)
                .chain(args.iter().map(String::as_str))
                .map(posix_shell_quote)
                .collect::<Vec<_>>()
                .join(" ");
            ssh_args.push(command);
            Ok(RunCommand {
                context_id: context.id.clone(),
                program: "ssh".into(),
                args: ssh_args,
                script: label,
                cwd: None,
                stdin: None,
                envs: crate::ssh_hosts::auth_envs_for_connection(&connection)?,
            })
        }
    }
}

fn command_failure_detail(output: &RunCommandOutput) -> String {
    let detail = if output.stderr.trim().is_empty() {
        output.stdout.trim()
    } else {
        output.stderr.trim()
    };
    if detail.is_empty() {
        "no diagnostic output".into()
    } else {
        tail(detail)
    }
}

pub fn build_run_command(
    ctx: &wisp_store::ExecutionContext,
    script: &str,
    cwd: Option<PathBuf>,
) -> RunCommand {
    let cfg: serde_json::Value = serde_json::from_str(&ctx.config_json).unwrap_or_default();
    match ctx.kind {
        wisp_store::ExecutionContextKind::Local => local_command(&ctx.id, script, cwd),
        wisp_store::ExecutionContextKind::Ssh => {
            let alias = cfg
                .get("alias")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| ctx.id.strip_prefix("ssh:").unwrap_or(&ctx.id));
            RunCommand {
                context_id: ctx.id.clone(),
                program: "ssh".into(),
                args: vec![alias.into(), script.into()],
                script: script.into(),
                cwd: None,
                stdin: None,
                envs: Vec::new(),
            }
        }
        wisp_store::ExecutionContextKind::Wsl => {
            let distro = cfg
                .get("distro")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| ctx.id.strip_prefix("wsl:").unwrap_or(&ctx.id));
            RunCommand {
                context_id: ctx.id.clone(),
                program: "wsl.exe".into(),
                args: vec![
                    "-d".into(),
                    distro.into(),
                    "--".into(),
                    "sh".into(),
                    "-lc".into(),
                    script.into(),
                ],
                script: script.into(),
                cwd: None,
                stdin: None,
                envs: Vec::new(),
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn local_command(context_id: &str, script: &str, cwd: Option<PathBuf>) -> RunCommand {
    RunCommand {
        context_id: context_id.into(),
        program: local_detached::windows_powershell_program().into(),
        args: vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            script.into(),
        ],
        script: script.into(),
        cwd,
        stdin: None,
        envs: wisp_tools::network::command_proxy_env(),
    }
}

#[cfg(not(target_os = "windows"))]
fn local_command(context_id: &str, script: &str, cwd: Option<PathBuf>) -> RunCommand {
    RunCommand {
        context_id: context_id.into(),
        program: "sh".into(),
        args: vec!["-lc".into(), script.into()],
        script: script.into(),
        cwd,
        stdin: None,
        envs: wisp_tools::network::command_proxy_env(),
    }
}

struct PreparedRun {
    run_id: String,
    project_id: String,
    command: RunCommand,
    timeout: Duration,
    output_specs: Vec<crate::harvest::OutputSpec>,
    frame_id: Option<String>,
    harvest_root: Option<PathBuf>,
    remote: Option<RemoteRun>,
    owner_id: String,
}

fn resolve_declared_inputs(
    root: &Path,
    refs: &[String],
    ssh_staging: bool,
) -> Result<Vec<PathBuf>, String> {
    if ssh_staging {
        return resolve_input_paths(root, refs);
    }
    refs.iter()
        .map(|reference| {
            let relative = Path::new(reference);
            if relative.as_os_str().is_empty()
                || relative.is_absolute()
                || relative.components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::ParentDir
                            | std::path::Component::RootDir
                            | std::path::Component::Prefix(_)
                    )
                })
            {
                return Err(format!("Run input must be project-relative: {reference}"));
            }
            let path = wisp_tools::safety::validate_file_path(root, reference)?;
            if !path.is_file() {
                return Err(format!("Run input is not a project file: {reference}"));
            }
            Ok(root.join(relative))
        })
        .collect()
}

fn git_code_state(root: Option<&Path>) -> (Option<String>, Option<String>) {
    const MAX_PATCH_BYTES: usize = 1024 * 1024;
    let Some(root) = root else {
        return (None, None);
    };
    let _git = wisp_tools::process::lock_git_command();
    let mut commit_cmd = std::process::Command::new("git");
    commit_cmd
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0");
    wisp_tools::process::hide_console(&mut commit_cmd);
    let commit = commit_cmd
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let dirty_patch = commit.as_ref().and_then(|_| {
        let mut diff_cmd = std::process::Command::new("git");
        diff_cmd
            .args(["diff", "--binary", "--no-ext-diff", "HEAD", "--", "."])
            .current_dir(root)
            .env("GIT_OPTIONAL_LOCKS", "0");
        wisp_tools::process::hide_console(&mut diff_cmd);
        diff_cmd
            .output()
            .ok()
            .filter(|output| output.status.success() && output.stdout.len() <= MAX_PATCH_BYTES)
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .filter(|value| !value.is_empty() && !contains_obvious_secret(value))
    });
    (commit, dirty_patch)
}

fn contains_obvious_secret(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase();
    [
        "-----begin private key-----",
        "-----begin rsa private key-----",
        "-----begin ec private key-----",
        "-----begin openssh private key-----",
        "authorization: bearer ",
        "x-api-key:",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

async fn record_created_run_lineage(
    store: &wisp_store::Store,
    run: &wisp_store::RunRecord,
    root: Option<&Path>,
    input_refs: &[String],
    input_paths: &[PathBuf],
    environment: &serde_json::Value,
) -> Result<(), String> {
    store
        .record_run_environment_snapshot(&run.id, Some(&run.context_id), environment)
        .await
        .map_err(|error| error.to_string())?;

    let command = run.command.as_deref().unwrap_or_default();
    let (git_commit, dirty_patch) = match root.map(|path| path.to_path_buf()) {
        Some(root) => tokio::task::spawn_blocking(move || git_code_state(Some(&root)))
            .await
            .map_err(|error| format!("git snapshot task failed: {error}"))?,
        None => (None, None),
    };
    store
        .save_run_code_snapshot(&wisp_store::RunCodeSnapshot {
            id: format!("run-code:{}:command", run.id),
            run_id: run.id.clone(),
            source_kind: "command".into(),
            source_path: run.script_path.clone(),
            source_text: command.to_string(),
            checksum: wisp_sync::sha256_hex(command.as_bytes()),
            storage_path: None,
            git_commit,
            dirty_patch,
            created_at: chrono::Utc::now().timestamp(),
        })
        .await
        .map_err(|error| error.to_string())?;

    if input_refs.is_empty() {
        return Ok(());
    }
    let root = root.ok_or_else(|| "Run inputs require a project root".to_string())?;
    let frame_id = run
        .frame_id
        .as_deref()
        .ok_or_else(|| "Run inputs require a source Session".to_string())?;
    for (source_ref, path) in input_refs.iter().zip(input_paths) {
        let source_ref = Path::new(source_ref)
            .components()
            .filter_map(|component| match component {
                std::path::Component::Normal(part) => Some(part),
                _ => None,
            })
            .collect::<PathBuf>()
            .to_string_lossy()
            .replace('\\', "/");
        let logical_key = format!("path:{source_ref}");
        let artifact_id = wisp_store::logical_artifact_id(&run.project_id, &logical_key);
        let captured = crate::snapshot_store::capture_file(
            root,
            path,
            crate::snapshot_store::SnapshotPolicy::UpTo(
                crate::snapshot_store::DEFAULT_SNAPSHOT_LIMIT,
            ),
        )?;
        let current = store
            .latest_artifact_version(&artifact_id)
            .await
            .map_err(|error| error.to_string())?;
        let version_id = if let Some(version) = current.as_ref().filter(|version| {
            version.checksum.as_deref() == Some(captured.checksum.as_str())
                && (version.materialization == captured.materialization
                    || version.materialization == wisp_store::ArtifactMaterialization::Snapshot)
        }) {
            version.id.clone()
        } else {
            let filename = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("input");
            store
                .save_artifact_version(&wisp_store::ArtifactVersionDraft {
                    version_id: None,
                    artifact_id,
                    project_id: run.project_id.clone(),
                    root_frame_id: frame_id.to_string(),
                    filename: filename.to_string(),
                    content_type: crate::mime::mime_for_path(path).to_string(),
                    storage_path: captured.storage_path,
                    logical_key: Some(logical_key),
                    size_bytes: Some(
                        i64::try_from(captured.size_bytes)
                            .map_err(|_| "Run input is too large".to_string())?,
                    ),
                    checksum: Some(captured.checksum),
                    producing_run_id: None,
                    env_snapshot_hash: None,
                    materialization: captured.materialization,
                    capture_timing: wisp_store::ArtifactCaptureTiming::AtCreation,
                })
                .await
                .map_err(|error| error.to_string())?
        };
        store
            .save_run_input(&wisp_store::RunInput {
                id: uuid::Uuid::new_v4().to_string(),
                run_id: run.id.clone(),
                artifact_version_id: Some(version_id),
                external_resource_id: None,
                source_ref,
                role: "input".into(),
                required: true,
                basis: wisp_store::LineageBasis::Declared,
                confidence: wisp_store::LineageConfidence::Exact,
                created_at: chrono::Utc::now().timestamp(),
            })
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

async fn create_run_record(
    store: &wisp_store::Store,
    project_id: &str,
    frame_id: Option<&str>,
    request: SubmitRunRequest,
    cwd: Option<PathBuf>,
    initial_status: wisp_store::RunStatus,
    owner_id: &str,
    lease_secs: i64,
    preflight: Option<&RunPreflightReport>,
) -> Result<PreparedRun, String> {
    let command = request.command.trim().to_string();
    if command.is_empty() {
        return Err("command is required".into());
    }
    let ctx = store
        .get_execution_context(&request.context_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Execution context not found: {}", request.context_id))?;
    if ctx.kind != wisp_store::ExecutionContextKind::Local {
        let selected = match frame_id {
            Some(frame_id) => store
                .session_execution_context_enabled(frame_id, &ctx.id)
                .await
                .map_err(|error| error.to_string())?,
            None => false,
        };
        if !selected {
            return Err(format!(
                "Execution context {} is not selected for this session",
                request.context_id
            ));
        }
    }
    if ctx.kind == wisp_store::ExecutionContextKind::Ssh {
        crate::ssh_hosts::require_managed_ssh_ready(&ctx)?;
    }
    let run_id = uuid::Uuid::new_v4().to_string();
    let output_specs = request.output_specs.unwrap_or_default();
    let input_refs = request.input_paths.unwrap_or_default();
    let input_paths = if input_refs.is_empty() {
        Vec::new()
    } else {
        let root = cwd
            .as_deref()
            .ok_or_else(|| "Run inputs require a project root".to_string())?;
        resolve_declared_inputs(
            root,
            &input_refs,
            ctx.kind == wisp_store::ExecutionContextKind::Ssh,
        )?
    };
    let timeout = Duration::from_secs(
        request
            .timeout_secs
            .unwrap_or(4 * 60 * 60)
            .clamp(1, 7 * 24 * 60 * 60),
    );
    let runner_kind = match ctx.kind {
        wisp_store::ExecutionContextKind::Ssh => "ssh_direct",
        wisp_store::ExecutionContextKind::Local | wisp_store::ExecutionContextKind::Wsl => {
            "local_detached"
        }
    };
    let mut run = wisp_store::RunRecord::new(
        &run_id,
        project_id,
        &ctx.id,
        request
            .title
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&command),
        runner_kind,
    );
    run.frame_id = frame_id.map(Into::into);
    run.command = Some(command.clone());
    run.input_refs_json = serde_json::to_string(&input_refs).map_err(|e| e.to_string())?;
    run.output_specs_json = serde_json::to_string(&output_specs).map_err(|e| e.to_string())?;
    run.timeout_secs = Some(timeout.as_secs() as i64);
    let environment = run_environment_snapshot(&ctx);
    let mut persisted_environment = environment.clone();
    persisted_environment["preflight"] = serde_json::to_value(preflight).unwrap_or_default();
    run.env_snapshot_json = wisp_store::canonical_json(&persisted_environment);

    let handle = match ctx.kind {
        wisp_store::ExecutionContextKind::Ssh => {
            for spec in output_specs
                .iter()
                .filter(|spec| !spec.glob.starts_with("ssh://"))
            {
                harvest_remote::validate_remote_glob(&spec.glob)?;
            }
            let (prefs, _) =
                crate::storage_prefs::effective_prefs(store, project_id, &ctx.id).await?;
            RemoteRunHandle::SshDirect {
                connection: crate::ssh_hosts::SshConnection::from_execution_context(&ctx)?,
                workdir: format!("{}/{run_id}", prefs.remote_workdir_root),
                token: uuid::Uuid::new_v4().to_string(),
                inputs_staged: false,
                pgid: None,
                start_time: None,
            }
        }
        wisp_store::ExecutionContextKind::Local | wisp_store::ExecutionContextKind::Wsl => {
            local_detached_handle_for(&ctx, &run_id, cwd.as_deref())?
        }
    };
    run.remote_workdir = Some(handle.display_workdir());
    run.remote_handle_json = Some(serde_json::to_string(&handle).map_err(|e| e.to_string())?);
    let remote = Some(RemoteRun {
        run_id: run_id.clone(),
        project_id: project_id.into(),
        frame_id: frame_id.map(Into::into),
        command: command.clone(),
        timeout,
        input_refs: input_refs.clone(),
        output_specs: output_specs.clone(),
        harvest_root: cwd.clone(),
        handle,
    });
    store.create_run(&run).await.map_err(|e| e.to_string())?;
    record_created_run_lineage(
        store,
        &run,
        cwd.as_deref(),
        &input_refs,
        &input_paths,
        &environment,
    )
    .await?;
    if !store
        .activate_run_lifecycle(&run_id, initial_status, owner_id, lease_secs)
        .await
        .map_err(|e| e.to_string())?
    {
        return Err("Run changed state before it could be activated".into());
    }
    Ok(PreparedRun {
        run_id,
        project_id: project_id.into(),
        command: build_run_command(&ctx, &command, cwd.clone()),
        timeout,
        output_specs,
        frame_id: frame_id.map(Into::into),
        harvest_root: cwd,
        remote,
        owner_id: owner_id.into(),
    })
}

fn local_detached_handle_for(
    ctx: &wisp_store::ExecutionContext,
    run_id: &str,
    cwd: Option<&Path>,
) -> Result<RemoteRunHandle, String> {
    let transport = match ctx.kind {
        wisp_store::ExecutionContextKind::Wsl => {
            let cfg: serde_json::Value = serde_json::from_str(&ctx.config_json).unwrap_or_default();
            let distro = cfg
                .get("distro")
                .and_then(|value| value.as_str())
                .unwrap_or_else(|| ctx.id.strip_prefix("wsl:").unwrap_or(&ctx.id));
            LocalTransport::Posix {
                context_id: ctx.id.clone(),
                program: "wsl.exe".into(),
                args: vec![
                    "-d".into(),
                    distro.into(),
                    "--".into(),
                    "sh".into(),
                    "-s".into(),
                ],
            }
        }
        wisp_store::ExecutionContextKind::Local => {
            #[cfg(windows)]
            {
                LocalTransport::Windows {
                    context_id: ctx.id.clone(),
                }
            }
            #[cfg(not(windows))]
            {
                LocalTransport::Posix {
                    context_id: ctx.id.clone(),
                    program: "sh".into(),
                    args: vec!["-s".into()],
                }
            }
        }
        wisp_store::ExecutionContextKind::Ssh => {
            return Err("local detached handle requires a local or WSL context".into());
        }
    };
    // Local commands run in the project root directly; WSL stores the Windows
    // project root and translates it through wslpath inside the distro.
    let command_cwd = cwd
        .map(|path| path.to_string_lossy().into_owned())
        .filter(|path| !path.is_empty());
    Ok(RemoteRunHandle::LocalDetached {
        transport,
        workdir: format!(".wisp-science/runs/{run_id}"),
        token: uuid::Uuid::new_v4().to_string(),
        inputs_staged: true,
        pgid: None,
        start_identity: None,
        command_cwd,
    })
}

async fn finish_remote_run(
    store: &wisp_store::Store,
    runner: &dyn RunCommandRunner,
    owner_id: &str,
    remote: &RemoteRun,
    status: wisp_store::RunStatus,
    exit_code: Option<i64>,
) -> Result<(), String> {
    with_run_lifecycle_lease(store, &remote.run_id, owner_id, async {
        if status == wisp_store::RunStatus::Succeeded && !remote.output_specs.is_empty() {
            if let Some(frame_id) = remote.frame_id.as_deref() {
                match harvest_finished_remote(store, runner, owner_id, remote, frame_id).await {
                    Ok(()) => {
                        store
                            .mark_run_harvested(&remote.run_id)
                            .await
                            .map_err(|e| e.to_string())?;
                    }
                    Err(error) => {
                        store
                            .record_run_poll_owned(
                                &remote.run_id,
                                owner_id,
                                None,
                                None,
                                Some(&format!("remote artifact registration failed: {error}")),
                            )
                            .await
                            .map_err(|e| e.to_string())?;
                    }
                }
            }
        }
        harvest_remote::require_owned_finish(
            store
                .finish_active_run_owned(&remote.run_id, owner_id, status, exit_code)
                .await
                .map_err(|e| e.to_string())?,
            "Run",
        )?;
        Ok(())
    })
    .await
}

/// Register a succeeded detached Run's declared outputs: local globs for
/// local/WSL Runs, and both `ssh://` URI references and glob-matched remote
/// pull-back for SSH-direct Runs.
async fn harvest_finished_remote(
    store: &wisp_store::Store,
    runner: &dyn RunCommandRunner,
    owner_id: &str,
    remote: &RemoteRun,
    frame_id: &str,
) -> Result<(), String> {
    let harvested_at = store
        .get_run(&remote.run_id)
        .await
        .map_err(|e| e.to_string())?
        .and_then(|run| run.harvested_at);
    if harvest_remote::skip_auto_harvest(harvested_at) {
        return Ok(());
    }
    let fallback = PathBuf::from(".");
    if remote.handle.is_local_detached() {
        let references: Vec<_> = remote
            .output_specs
            .iter()
            .filter(|spec| !spec.glob.starts_with("ssh://"))
            .cloned()
            .collect();
        if !references.is_empty() {
            crate::harvest::harvest_run_outputs(
                store,
                &remote.project_id,
                frame_id,
                &remote.run_id,
                remote.harvest_root.as_deref().unwrap_or(&fallback),
                &references,
            )
            .await?;
        }
        return Ok(());
    }
    let uri_references: Vec<_> = remote
        .output_specs
        .iter()
        .filter(|spec| spec.glob.starts_with("ssh://"))
        .cloned()
        .collect();
    if !uri_references.is_empty() {
        crate::harvest::harvest_run_outputs(
            store,
            &remote.project_id,
            frame_id,
            &remote.run_id,
            remote.harvest_root.as_deref().unwrap_or(&fallback),
            &uri_references,
        )
        .await?;
    }
    harvest_remote::harvest_ssh_run(store, runner, owner_id, remote, true).await?;
    Ok(())
}

fn retry_stopped_error(handle: &RemoteRunHandle, error: &str) -> String {
    let marker = if handle.is_local_detached() {
        LOCAL_RETRY_STOPPED_MARKER
    } else {
        SSH_RETRY_STOPPED_MARKER
    };
    if error.contains(marker) {
        return error.to_string();
    }
    if handle.is_local_detached() {
        format!("{marker} after the first failed start attempt. Manual retry is required. {error}")
    } else {
        format!(
            "{marker} after the first failed attempt to protect the server. Manual retry is required. {error}"
        )
    }
}

fn remote_lifecycle_lease_secs(remote: &RemoteRun) -> i64 {
    if remote.handle.is_confirmed() {
        ACTIVE_LEASE_SECS
    } else {
        REMOTE_START_LEASE_SECS
    }
}

async fn fail_remote_start(
    store: &wisp_store::Store,
    runner: &dyn RunCommandRunner,
    owner_id: &str,
    remote: &RemoteRun,
    error: &str,
) -> Result<(), String> {
    let error_tail = tail(error);
    store
        .record_run_poll_owned(&remote.run_id, owner_id, None, None, Some(error))
        .await
        .map_err(|e| e.to_string())?;
    store
        .update_run_output_owned(&remote.run_id, owner_id, None, Some(&error_tail))
        .await
        .map_err(|e| e.to_string())?;
    finish_remote_run(
        store,
        runner,
        owner_id,
        remote,
        wisp_store::RunStatus::Failed,
        Some(69),
    )
    .await
}

async fn remote_lifecycle(
    store: &wisp_store::Store,
    runner: &dyn RunCommandRunner,
    owner_id: &str,
    mut remote: RemoteRun,
) -> Result<(), String> {
    let mut consecutive_transport_errors = 0_u32;
    loop {
        let lease_secs = remote_lifecycle_lease_secs(&remote);
        if !store
            .renew_run_lifecycle(&remote.run_id, owner_id, lease_secs)
            .await
            .map_err(|e| e.to_string())?
        {
            return Ok(());
        }
        let run = store
            .get_run(&remote.run_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Run not found: {}", remote.run_id))?;
        if run.status.is_terminal() {
            return Ok(());
        }

        if !remote.handle.is_confirmed()
            && run.status == wisp_store::RunStatus::Submitted
            && run.last_poll_error.is_some()
        {
            let error = retry_stopped_error(
                &remote.handle,
                run.last_poll_error
                    .as_deref()
                    .unwrap_or("unknown start error"),
            );
            fail_remote_start(store, runner, owner_id, &remote, &error).await?;
            return Ok(());
        }

        if run.status == wisp_store::RunStatus::Submitted && remote.handle.is_confirmed() {
            let _ = store
                .transition_run_to_running_owned(&remote.run_id, owner_id)
                .await
                .map_err(|e| e.to_string())?;
            continue;
        }

        if !remote.handle.is_confirmed() {
            if run.status == wisp_store::RunStatus::Cancelling {
                match prepare_remote(runner, &remote).await {
                    Ok(PrepareRemote::Existing(handle)) => remote.handle = handle,
                    Ok(PrepareRemote::Prepared) => {
                        finish_remote_run(
                            store,
                            runner,
                            owner_id,
                            &remote,
                            wisp_store::RunStatus::Cancelled,
                            None,
                        )
                        .await?;
                        return Ok(());
                    }
                    Err(error) => {
                        store
                            .record_run_poll_owned(
                                &remote.run_id,
                                owner_id,
                                None,
                                None,
                                Some(&error),
                            )
                            .await
                            .map_err(|e| e.to_string())?;
                        finish_remote_run(
                            store,
                            runner,
                            owner_id,
                            &remote,
                            wisp_store::RunStatus::Cancelled,
                            None,
                        )
                        .await?;
                        return Ok(());
                    }
                }
            } else {
                match ensure_remote_started(store, owner_id, runner, &mut remote).await {
                    Ok(handle) => remote.handle = handle,
                    Err(error) => {
                        let error = retry_stopped_error(&remote.handle, &error);
                        fail_remote_start(store, runner, owner_id, &remote, &error).await?;
                        return Ok(());
                    }
                }
            }
            let handle_json = serde_json::to_string(&remote.handle).map_err(|e| e.to_string())?;
            store
                .set_run_remote_handle_owned(
                    &remote.run_id,
                    owner_id,
                    &handle_json,
                    &remote.handle.display_workdir(),
                )
                .await
                .map_err(|e| e.to_string())?;
            let refreshed = store
                .get_run(&remote.run_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Run not found: {}", remote.run_id))?;
            if refreshed.status == wisp_store::RunStatus::Submitted {
                store
                    .transition_run_to_running_owned(&remote.run_id, owner_id)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            continue;
        }

        if run.status == wisp_store::RunStatus::Cancelling {
            match cancel_remote(runner, &remote.handle).await {
                Ok(RemoteCancel::Cancelled) => {
                    finish_remote_run(
                        store,
                        runner,
                        owner_id,
                        &remote,
                        wisp_store::RunStatus::Cancelled,
                        None,
                    )
                    .await?;
                    return Ok(());
                }
                Ok(RemoteCancel::Finished(code)) => {
                    finish_remote_run(
                        store,
                        runner,
                        owner_id,
                        &remote,
                        remote_terminal_status(code),
                        Some(code),
                    )
                    .await?;
                    return Ok(());
                }
                Ok(RemoteCancel::TimedOut(code)) => {
                    finish_remote_run(
                        store,
                        runner,
                        owner_id,
                        &remote,
                        wisp_store::RunStatus::TimedOut,
                        Some(code),
                    )
                    .await?;
                    return Ok(());
                }
                Ok(RemoteCancel::Lost(reason)) => {
                    store
                        .record_run_poll_owned(&remote.run_id, owner_id, None, None, Some(&reason))
                        .await
                        .map_err(|e| e.to_string())?;
                    finish_remote_run(
                        store,
                        runner,
                        owner_id,
                        &remote,
                        wisp_store::RunStatus::Lost,
                        None,
                    )
                    .await?;
                    return Ok(());
                }
                Err(error) => {
                    if permanent_remote_start_error(&error) {
                        let error = retry_stopped_error(&remote.handle, &error);
                        store
                            .record_run_poll_owned(
                                &remote.run_id,
                                owner_id,
                                None,
                                None,
                                Some(&error),
                            )
                            .await
                            .map_err(|e| e.to_string())?;
                        finish_remote_run(
                            store,
                            runner,
                            owner_id,
                            &remote,
                            wisp_store::RunStatus::Lost,
                            None,
                        )
                        .await?;
                        return Ok(());
                    }
                    store
                        .record_run_poll_owned(&remote.run_id, owner_id, None, None, Some(&error))
                        .await
                        .map_err(|e| e.to_string())?;
                    consecutive_transport_errors = consecutive_transport_errors.saturating_add(1);
                }
            }
        } else {
            match poll_remote(runner, &remote.handle).await {
                Ok(poll) => {
                    consecutive_transport_errors = 0;
                    store
                        .record_run_poll_owned(
                            &remote.run_id,
                            owner_id,
                            Some(&tail(&poll.stdout)),
                            Some(&tail(&poll.stderr)),
                            None,
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                    match poll.state {
                        RemotePollState::Running => {}
                        RemotePollState::Finished(code) => {
                            finish_remote_run(
                                store,
                                runner,
                                owner_id,
                                &remote,
                                remote_terminal_status(code),
                                Some(code),
                            )
                            .await?;
                            return Ok(());
                        }
                        RemotePollState::TimedOut(code) => {
                            finish_remote_run(
                                store,
                                runner,
                                owner_id,
                                &remote,
                                wisp_store::RunStatus::TimedOut,
                                Some(code),
                            )
                            .await?;
                            return Ok(());
                        }
                        RemotePollState::Cancelled => {
                            finish_remote_run(
                                store,
                                runner,
                                owner_id,
                                &remote,
                                wisp_store::RunStatus::Cancelled,
                                None,
                            )
                            .await?;
                            return Ok(());
                        }
                        RemotePollState::Lost(reason) => {
                            store
                                .record_run_poll_owned(
                                    &remote.run_id,
                                    owner_id,
                                    None,
                                    None,
                                    Some(&reason),
                                )
                                .await
                                .map_err(|e| e.to_string())?;
                            finish_remote_run(
                                store,
                                runner,
                                owner_id,
                                &remote,
                                wisp_store::RunStatus::Lost,
                                None,
                            )
                            .await?;
                            return Ok(());
                        }
                    }
                }
                Err(error) => {
                    if permanent_remote_start_error(&error) {
                        let error = retry_stopped_error(&remote.handle, &error);
                        store
                            .record_run_poll_owned(
                                &remote.run_id,
                                owner_id,
                                None,
                                None,
                                Some(&error),
                            )
                            .await
                            .map_err(|e| e.to_string())?;
                        finish_remote_run(
                            store,
                            runner,
                            owner_id,
                            &remote,
                            wisp_store::RunStatus::Lost,
                            None,
                        )
                        .await?;
                        return Ok(());
                    }
                    // The process is confirmed on the server, so temporary transport
                    // failures retain the handle and back off instead of relaunching.
                    store
                        .record_run_poll_owned(&remote.run_id, owner_id, None, None, Some(&error))
                        .await
                        .map_err(|e| e.to_string())?;
                    consecutive_transport_errors = consecutive_transport_errors.saturating_add(1);
                }
            }
        }
        tokio::time::sleep(remote_poll_interval_for(
            &remote.handle,
            consecutive_transport_errors,
        ))
        .await;
    }
}

/// Pull a trailing slice of the run's stdout/stderr logs (capped, not the
/// full remote files) into `<workspace>/runs/<run_id>/` so the server
/// workspace can be deleted without losing the useful tail. Returns the
/// project-relative log directory when anything was written.
async fn save_run_logs_locally(
    store: &wisp_store::Store,
    runner: &dyn RunCommandRunner,
    run: &wisp_store::RunRecord,
    handle: &RemoteRunHandle,
) -> Result<Option<String>, String> {
    let Some(workspace) = store
        .get_project(&run.project_id)
        .await
        .map_err(|e| e.to_string())?
        .map(|(_, workspace)| workspace)
        .filter(|workspace| !workspace.trim().is_empty())
    else {
        return Ok(None);
    };
    let pull = cleanup::fetch_run_logs(runner, handle, &run.id).await?;
    let (stdout_log, stderr_log) = match pull {
        cleanup::LogPull::Absent => return Ok(None),
        cleanup::LogPull::EncoderMissing => {
            // The server cannot emit binary-safe output; the persisted tails
            // are the best remaining copy.
            let as_log = |tail: &Option<String>| {
                tail.as_deref()
                    .filter(|tail| !tail.is_empty())
                    .map(|tail| cleanup::PulledLog {
                        total_size: tail.len() as u64,
                        bytes: tail.as_bytes().to_vec(),
                    })
            };
            (as_log(&run.stdout_tail), as_log(&run.stderr_tail))
        }
        cleanup::LogPull::Logs { stdout, stderr } => (stdout, stderr),
    };
    if stdout_log.is_none() && stderr_log.is_none() {
        return Ok(None);
    }
    let relative = format!("runs/{}", run.id);
    let directory = Path::new(&workspace).join("runs").join(&run.id);
    std::fs::create_dir_all(&directory).map_err(|e| format!("create local log directory: {e}"))?;
    for (name, log) in [("stdout.log", stdout_log), ("stderr.log", stderr_log)] {
        let Some(log) = log else { continue };
        let path = directory.join(name);
        let mut contents = Vec::with_capacity(log.bytes.len() + 96);
        if (log.bytes.len() as u64) < log.total_size {
            contents.extend_from_slice(
                format!(
                    "[wisp] log truncated: showing last {} of {} bytes\n",
                    log.bytes.len(),
                    log.total_size
                )
                .as_bytes(),
            );
        }
        contents.extend_from_slice(&log.bytes);
        std::fs::write(&path, contents).map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    Ok(Some(relative))
}

async fn remote_run_from_record(
    store: &wisp_store::Store,
    run: &wisp_store::RunRecord,
) -> Result<Option<RemoteRun>, String> {
    let Some(handle_json) = run.remote_handle_json.as_deref() else {
        return Ok(None);
    };
    let handle: RemoteRunHandle = serde_json::from_str(handle_json)
        .map_err(|e| format!("Run {} has an invalid remote handle: {e}", run.id))?;
    let workspace = store
        .get_project(&run.project_id)
        .await
        .map_err(|e| e.to_string())?
        .map(|(_, workspace)| workspace)
        .filter(|workspace| !workspace.trim().is_empty())
        .map(PathBuf::from);
    let input_refs: Vec<String> = serde_json::from_str(&run.input_refs_json)
        .map_err(|e| format!("Run {} has invalid input refs: {e}", run.id))?;
    Ok(Some(RemoteRun {
        run_id: run.id.clone(),
        project_id: run.project_id.clone(),
        frame_id: run.frame_id.clone(),
        command: run
            .command
            .clone()
            .ok_or_else(|| format!("SSH Run {} has no command", run.id))?,
        timeout: Duration::from_secs(run.timeout_secs.unwrap_or(4 * 60 * 60) as u64),
        input_refs,
        output_specs: serde_json::from_str(&run.output_specs_json)
            .map_err(|e| format!("Run {} has invalid output specs: {e}", run.id))?,
        harvest_root: workspace,
        handle,
    }))
}

fn response_from_run(run: &wisp_store::RunRecord) -> SubmitRunResponse {
    SubmitRunResponse {
        run_id: run.id.clone(),
        status: run.status,
        exit_code: run.exit_code,
        stdout_tail: run.stdout_tail.clone(),
        stderr_tail: run.stderr_tail.clone(),
        remote_workdir: run.remote_workdir.clone(),
    }
}

async fn record_run_outcome(
    store: &wisp_store::Store,
    prepared: &PreparedRun,
    output: Result<RunCommandOutput, String>,
    owner_id: &str,
) -> Result<SubmitRunResponse, String> {
    match output {
        Ok(out) => {
            let stdout_tail = tail(&out.stdout);
            let stderr_tail = tail(&out.stderr);
            store
                .update_run_output_owned(
                    &prepared.run_id,
                    owner_id,
                    Some(&stdout_tail),
                    Some(&stderr_tail),
                )
                .await
                .map_err(|e| e.to_string())?;
            let status = if out.exit_code == 0 {
                wisp_store::RunStatus::Succeeded
            } else {
                wisp_store::RunStatus::Failed
            };
            store
                .finish_active_run_owned(&prepared.run_id, owner_id, status, Some(out.exit_code))
                .await
                .map_err(|e| e.to_string())?;
            if status == wisp_store::RunStatus::Succeeded {
                if let (Some(frame_id), Some(root)) = (
                    prepared.frame_id.as_deref(),
                    prepared.harvest_root.as_deref(),
                ) {
                    if !prepared.output_specs.is_empty() {
                        crate::harvest::harvest_run_outputs(
                            store,
                            &prepared.project_id,
                            frame_id,
                            &prepared.run_id,
                            root,
                            &prepared.output_specs,
                        )
                        .await?;
                        store
                            .mark_run_harvested(&prepared.run_id)
                            .await
                            .map_err(|e| e.to_string())?;
                    }
                }
            }
            Ok(SubmitRunResponse {
                run_id: prepared.run_id.clone(),
                status,
                exit_code: Some(out.exit_code),
                stdout_tail: Some(stdout_tail),
                stderr_tail: Some(stderr_tail),
                remote_workdir: None,
            })
        }
        Err(e) => {
            let stderr_tail = tail(&e);
            store
                .update_run_output_owned(&prepared.run_id, owner_id, None, Some(&stderr_tail))
                .await
                .map_err(|err| err.to_string())?;
            let (status, exit_code) = if e == "run_in_context cancelled" {
                (wisp_store::RunStatus::Cancelled, None)
            } else if e.starts_with("run_in_context timed out after ") {
                (wisp_store::RunStatus::TimedOut, Some(124))
            } else {
                (wisp_store::RunStatus::Failed, Some(-1))
            };
            store
                .finish_active_run_owned(&prepared.run_id, owner_id, status, exit_code)
                .await
                .map_err(|err| err.to_string())?;
            Ok(SubmitRunResponse {
                run_id: prepared.run_id.clone(),
                status,
                exit_code,
                stdout_tail: None,
                stderr_tail: Some(stderr_tail),
                remote_workdir: None,
            })
        }
    }
}

/// Keep `run_id`'s lifecycle lease alive until `operation` completes.
/// Renew runs on its own task so a store write inside `operation` is not
/// paused while another connection tries to renew (that deadlocks SQLite).
/// Renew failure is a hard error so a long harvest cannot outlive its owner.
async fn with_run_lifecycle_lease<T>(
    store: &wisp_store::Store,
    run_id: &str,
    owner_id: &str,
    operation: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    let store = store.clone();
    let run_id = run_id.to_string();
    let owner_id = owner_id.to_string();
    let mut renew = tokio::spawn(async move {
        let mut interval = tokio::time::interval(harvest_remote::harvest_lease_interval());
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            harvest_remote::require_lease_renewed(
                store
                    .renew_run_lifecycle(&run_id, &owner_id, ACTIVE_LEASE_SECS)
                    .await
                    .map_err(|e| e.to_string())?,
            )?;
        }
        #[allow(unreachable_code)]
        Ok::<(), String>(())
    });
    tokio::pin!(operation);
    tokio::select! {
        result = &mut operation => {
            renew.abort();
            let _ = renew.await;
            result
        }
        renew_result = &mut renew => {
            match renew_result {
                Ok(Err(error)) => Err(error),
                Ok(Ok(())) => Err("Run lifecycle lease renewer exited".into()),
                Err(join) if join.is_cancelled() => {
                    Err("Run lifecycle lease renewer cancelled".into())
                }
                Err(join) => Err(join.to_string()),
            }
        }
    }
}

async fn run_with_lifecycle_lease(
    store: &wisp_store::Store,
    run_id: &str,
    owner_id: &str,
    runner: &dyn RunCommandRunner,
    command: RunCommand,
    timeout: Duration,
) -> Result<RunCommandOutput, String> {
    let (updates_tx, mut updates_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut operation = Box::pin(runner.run_streaming(command, timeout, updates_tx));
    let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut output_flush = tokio::time::interval(Duration::from_secs(1));
    output_flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut stdout_tail = Vec::with_capacity(MAX_RUN_OUTPUT_BYTES);
    let mut stderr_tail = Vec::with_capacity(MAX_RUN_OUTPUT_BYTES);
    let mut output_dirty = false;
    let mut updates_open = true;
    loop {
        tokio::select! {
            output = &mut operation => return output,
            update = updates_rx.recv(), if updates_open => {
                match update {
                    Some(update) => {
                        let target = match update.stream {
                            RunOutputStream::Stdout => &mut stdout_tail,
                            RunOutputStream::Stderr => &mut stderr_tail,
                        };
                        append_tail_bytes(target, &update.chunk);
                        output_dirty = true;
                    }
                    None => updates_open = false,
                }
            }
            _ = output_flush.tick(), if output_dirty => {
                let stdout = String::from_utf8_lossy(&stdout_tail);
                let stderr = String::from_utf8_lossy(&stderr_tail);
                let owned = store
                    .record_run_poll_owned(
                        run_id,
                        owner_id,
                        (!stdout.is_empty()).then_some(stdout.as_ref()),
                        (!stderr.is_empty()).then_some(stderr.as_ref()),
                        None,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                if !owned {
                    return Err("Run lifecycle lease was lost".into());
                }
                output_dirty = false;
            }
            _ = heartbeat.tick() => {
                let status = store
                    .get_run(run_id)
                    .await
                    .map_err(|e| e.to_string())?
                    .map(|run| run.status);
                if status == Some(wisp_store::RunStatus::Cancelling) {
                    return Err("run_in_context cancelled".into());
                }
                let owned = store
                    .renew_run_lifecycle(run_id, owner_id, ACTIVE_LEASE_SECS)
                    .await
                    .map_err(|e| e.to_string())?;
                if !owned {
                    return Err("Run lifecycle lease was lost".into());
                }
                let owned = store
                    .record_run_poll_owned(run_id, owner_id, None, None, None)
                    .await
                    .map_err(|e| e.to_string())?;
                if !owned {
                    return Err("Run lifecycle lease was lost".into());
                }
            }
        }
    }
}

#[cfg(test)]
pub async fn submit_run_with_runner(
    store: &wisp_store::Store,
    project_id: &str,
    frame_id: Option<&str>,
    request: SubmitRunRequest,
    runner: &dyn RunCommandRunner,
    cwd: Option<PathBuf>,
) -> Result<SubmitRunResponse, String> {
    let prepared = create_run_record(
        store,
        project_id,
        frame_id,
        request,
        cwd,
        wisp_store::RunStatus::Running,
        "test-owner",
        ACTIVE_LEASE_SECS,
        None,
    )
    .await?;
    let output = runner.run(prepared.command.clone(), prepared.timeout).await;
    record_run_outcome(store, &prepared, output, "test-owner").await
}

fn tail(s: &str) -> String {
    const MAX: usize = 4000;
    if s.len() <= MAX {
        s.to_string()
    } else {
        let mut start = s.len() - MAX;
        while !s.is_char_boundary(start) {
            start += 1;
        }
        s[start..].to_string()
    }
}

#[cfg(test)]
mod tests;
