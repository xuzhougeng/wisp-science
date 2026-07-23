use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::Mutex;

mod remote;
mod tools;
mod transfer;

#[cfg(all(test, windows))]
use remote::scp_local_path;
#[cfg(all(test, unix))]
use remote::{cancel_payload, launch_payload, poll_payload, prepare_payload};
use remote::{
    cancel_remote, checked_output, ensure_remote_started, permanent_remote_start_error,
    poll_remote, prepare_remote, remote_poll_interval, remote_terminal_status, resolve_input_paths,
    ssh_script_command, PrepareRemote, RemoteCancel, RemotePollState,
};
#[cfg(test)]
use remote::{parse_input_progress, remote_poll_delay_secs};
pub use tools::{CancelRunTool, GetRunTool, MonitorRunTool, RunInContextTool};
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

#[async_trait::async_trait]
pub trait RunCommandRunner: Send + Sync {
    async fn run(&self, command: RunCommand, timeout: Duration)
        -> Result<RunCommandOutput, String>;
}

#[derive(Clone)]
pub(crate) struct ProcessRunRunner;

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

async fn read_tail<R: AsyncRead + Unpin>(mut reader: R) -> std::io::Result<Vec<u8>> {
    let mut tail = Vec::with_capacity(MAX_RUN_OUTPUT_BYTES);
    let mut chunk = [0_u8; 8192];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            return Ok(tail);
        }
        if read >= MAX_RUN_OUTPUT_BYTES {
            tail.clear();
            tail.extend_from_slice(&chunk[read - MAX_RUN_OUTPUT_BYTES..read]);
            continue;
        }
        let overflow = (tail.len() + read).saturating_sub(MAX_RUN_OUTPUT_BYTES);
        if overflow > 0 {
            tail.drain(..overflow);
        }
        tail.extend_from_slice(&chunk[..read]);
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
            if crate::ssh_guard::is_connectivity_failure(detail) {
                crate::ssh_guard::record_failure(context_id, detail);
            }
        }
        Err(error) => {
            if crate::ssh_guard::is_connectivity_failure(error) {
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
        let mut stdout_task = tokio::spawn(read_tail(stdout));
        let mut stderr_task = tokio::spawn(read_tail(stderr));
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
}

const REMOTE_RPC_TIMEOUT: Duration = Duration::from_secs(20);

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
}

impl RemoteRunHandle {
    fn is_confirmed(&self) -> bool {
        match self {
            Self::SshDirect {
                pgid, start_time, ..
            } => pgid.is_some() && start_time.is_some(),
        }
    }

    fn inputs_staged(&self) -> bool {
        match self {
            Self::SshDirect { inputs_staged, .. } => *inputs_staged,
        }
    }

    fn mark_inputs_staged(&mut self) {
        match self {
            Self::SshDirect { inputs_staged, .. } => *inputs_staged = true,
        }
    }

    fn display_workdir(&self) -> String {
        match self {
            Self::SshDirect { workdir, .. } => format!("~/{workdir}"),
        }
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
}

const REMOTE_START_LEASE_SECS: i64 = 360;
const ACTIVE_LEASE_SECS: i64 = 30;
const RECONCILE_INTERVAL: Duration = Duration::from_secs(5);
const SSH_RETRY_STOPPED_MARKER: &str = "SSH automatic retry stopped";

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

    pub fn with_runner(runner: Arc<dyn RunCommandRunner>) -> Self {
        Self {
            runner,
            active: Arc::new(Mutex::new(HashMap::new())),
            owner_id: uuid::Uuid::new_v4().to_string(),
            reconciler_started: Arc::new(AtomicBool::new(false)),
        }
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

    async fn reconcile_once(&self, store: &wisp_store::Store) -> Result<u64, String> {
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
        let prepared = create_run_record(
            &store,
            &project_id,
            frame_id.as_deref(),
            request,
            cwd,
            wisp_store::RunStatus::Submitted,
            &self.owner_id,
            REMOTE_START_LEASE_SECS,
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

    pub async fn cancel(&self, store: &wisp_store::Store, run_id: &str) -> Result<(), String> {
        let run = store
            .get_run(run_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Run not found: {run_id}"))?;
        if run.status.is_terminal() {
            return Err(format!("Run is already {}", run.status.as_str()));
        }
        let requested = if run.status == wisp_store::RunStatus::Cancelling {
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
            if requested {
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
        if requested {
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
        program: "powershell".into(),
        args: vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            script.into(),
        ],
        script: script.into(),
        cwd,
        stdin: None,
        envs: Vec::new(),
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
        envs: Vec::new(),
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

async fn create_run_record(
    store: &wisp_store::Store,
    project_id: &str,
    frame_id: Option<&str>,
    request: SubmitRunRequest,
    cwd: Option<PathBuf>,
    initial_status: wisp_store::RunStatus,
    owner_id: &str,
    lease_secs: i64,
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
    let timeout = match ctx.kind {
        wisp_store::ExecutionContextKind::Ssh => Duration::from_secs(
            request
                .timeout_secs
                .unwrap_or(4 * 60 * 60)
                .clamp(1, 7 * 24 * 60 * 60),
        ),
        _ => Duration::from_secs(request.timeout_secs.unwrap_or(60).clamp(1, 300)),
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
        if ctx.kind == wisp_store::ExecutionContextKind::Ssh {
            "ssh_direct"
        } else {
            "command"
        },
    );
    run.frame_id = frame_id.map(Into::into);
    run.command = Some(command.clone());
    run.input_refs_json = serde_json::to_string(&input_refs).map_err(|e| e.to_string())?;
    run.output_specs_json = serde_json::to_string(&output_specs).map_err(|e| e.to_string())?;
    run.timeout_secs = Some(timeout.as_secs() as i64);
    run.env_snapshot_json = serde_json::json!({
        "context_id": ctx.id,
        "config": serde_json::from_str::<serde_json::Value>(&ctx.config_json).unwrap_or_default(),
        "capabilities": serde_json::from_str::<serde_json::Value>(&ctx.capabilities_json).unwrap_or_default(),
    })
    .to_string();

    let remote = if ctx.kind == wisp_store::ExecutionContextKind::Ssh {
        if output_specs
            .iter()
            .any(|spec| !spec.glob.starts_with("ssh://"))
        {
            return Err(
                "SSH direct output_specs must be explicit ssh:// references; remote glob harvest is not available yet"
                    .into(),
            );
        }
        let root = cwd
            .as_deref()
            .ok_or_else(|| "SSH input staging requires a project root".to_string())?;
        resolve_input_paths(root, &input_refs)?;
        let handle = RemoteRunHandle::SshDirect {
            connection: crate::ssh_hosts::SshConnection::from_execution_context(&ctx)?,
            workdir: format!(".wisp-science/runs/{run_id}"),
            token: uuid::Uuid::new_v4().to_string(),
            inputs_staged: false,
            pgid: None,
            start_time: None,
        };
        run.remote_workdir = Some(handle.display_workdir());
        run.remote_handle_json = Some(serde_json::to_string(&handle).map_err(|e| e.to_string())?);
        Some(RemoteRun {
            run_id: run_id.clone(),
            project_id: project_id.into(),
            frame_id: frame_id.map(Into::into),
            command: command.clone(),
            timeout,
            input_refs: input_refs.clone(),
            output_specs: output_specs.clone(),
            harvest_root: cwd.clone(),
            handle,
        })
    } else {
        if !input_refs.is_empty() {
            return Err("input_paths is only supported for SSH execution contexts".into());
        }
        None
    };
    store.create_run(&run).await.map_err(|e| e.to_string())?;
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

async fn finish_remote_run(
    store: &wisp_store::Store,
    owner_id: &str,
    remote: &RemoteRun,
    status: wisp_store::RunStatus,
    exit_code: Option<i64>,
) -> Result<(), String> {
    if status == wisp_store::RunStatus::Succeeded {
        if let Some(frame_id) = remote.frame_id.as_deref() {
            let references: Vec<_> = remote
                .output_specs
                .iter()
                .filter(|spec| spec.glob.starts_with("ssh://"))
                .cloned()
                .collect();
            if !references.is_empty() {
                let fallback = PathBuf::from(".");
                if let Err(error) = crate::harvest::harvest_run_outputs(
                    store,
                    &remote.project_id,
                    frame_id,
                    &remote.run_id,
                    remote.harvest_root.as_deref().unwrap_or(&fallback),
                    &references,
                )
                .await
                {
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
    let _ = store
        .finish_active_run_owned(&remote.run_id, owner_id, status, exit_code)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn ssh_retry_stopped_error(error: &str) -> String {
    if error.contains(SSH_RETRY_STOPPED_MARKER) {
        error.to_string()
    } else {
        format!(
            "{SSH_RETRY_STOPPED_MARKER} after the first failed attempt to protect the server. Manual retry is required. {error}"
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
            let error = ssh_retry_stopped_error(
                run.last_poll_error
                    .as_deref()
                    .unwrap_or("unknown SSH error"),
            );
            fail_remote_start(store, owner_id, &remote, &error).await?;
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
                        let error = ssh_retry_stopped_error(&error);
                        fail_remote_start(store, owner_id, &remote, &error).await?;
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
                    finish_remote_run(store, owner_id, &remote, wisp_store::RunStatus::Lost, None)
                        .await?;
                    return Ok(());
                }
                Err(error) => {
                    if permanent_remote_start_error(&error) {
                        let error = ssh_retry_stopped_error(&error);
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
                        let error = ssh_retry_stopped_error(&error);
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
        tokio::time::sleep(remote_poll_interval(consecutive_transport_errors)).await;
    }
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

async fn run_with_lifecycle_lease(
    store: &wisp_store::Store,
    run_id: &str,
    owner_id: &str,
    runner: &dyn RunCommandRunner,
    command: RunCommand,
    timeout: Duration,
) -> Result<RunCommandOutput, String> {
    let mut operation = Box::pin(runner.run(command, timeout));
    let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
    loop {
        tokio::select! {
            output = &mut operation => return output,
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
