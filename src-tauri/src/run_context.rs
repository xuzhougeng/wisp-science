use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::Mutex;

mod remote;
mod tools;

#[cfg(all(test, unix))]
use remote::{cancel_payload, launch_payload, poll_payload, prepare_payload};
use remote::{
    cancel_remote, ensure_remote_started, permanent_remote_start_error, poll_remote,
    prepare_remote, remote_poll_interval, remote_terminal_status, resolve_input_paths,
    PrepareRemote, RemoteCancel, RemotePollState,
};
pub use tools::{CancelRunTool, GetRunTool, RunInContextTool};

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
struct ProcessRunRunner;

#[async_trait::async_trait]
impl RunCommandRunner for ProcessRunRunner {
    async fn run(
        &self,
        command: RunCommand,
        timeout: Duration,
    ) -> Result<RunCommandOutput, String> {
        let mut cmd = Command::new(&command.program);
        cmd.args(&command.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
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
        let operation = async move {
            if let Some(input) = command.stdin {
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
            child
                .wait_with_output()
                .await
                .map_err(|e| format!("run_in_context wait failed: {e}"))
        };
        let output = tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| format!("run_in_context timed out after {}s", timeout.as_secs()))??;
        Ok(RunCommandOutput {
            exit_code: output.status.code().unwrap_or(-1) as i64,
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
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
        context: &wisp_store::ExecutionContext,
        remote_path: &str,
        destination: &std::path::Path,
    ) -> Result<(), String> {
        if remote_path.is_empty() || remote_path.contains(['\0', '\n', '\r']) {
            return Err("Invalid remote file path".into());
        }
        let connection = crate::ssh_hosts::SshConnection::from_execution_context(context)?;
        let mut args = connection.scp_option_args()?;
        args.push(format!("{}:{remote_path}", connection.target()?));
        args.push(destination.to_string_lossy().into_owned());
        let output = self
            .runner
            .run(
                RunCommand {
                    context_id: context.id.clone(),
                    program: "scp".into(),
                    args,
                    script: format!("download {remote_path}"),
                    cwd: destination.parent().map(std::path::Path::to_path_buf),
                    stdin: None,
                },
                Duration::from_secs(4 * 60 * 60),
            )
            .await?;
        if output.exit_code == 0 {
            Ok(())
        } else {
            let detail = if output.stderr.trim().is_empty() {
                output.stdout.trim()
            } else {
                output.stderr.trim()
            };
            Err(format!(
                "scp download failed (exit {}): {detail}",
                output.exit_code
            ))
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
        let claimed = store
            .claim_run_lifecycle(&remote.run_id, &self.owner_id, ACTIVE_LEASE_SECS)
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
                        tokio::time::sleep(remote_poll_interval()).await;
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
            return Err(format!("Run is already {}", refreshed.status.as_str()));
        }
        if let Some(remote) = remote_run_from_record(store, &refreshed).await? {
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

async fn remote_lifecycle(
    store: &wisp_store::Store,
    runner: &dyn RunCommandRunner,
    owner_id: &str,
    mut remote: RemoteRun,
) -> Result<(), String> {
    loop {
        if !store
            .renew_run_lifecycle(&remote.run_id, owner_id, ACTIVE_LEASE_SECS)
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
                        if permanent_remote_start_error(&error) {
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
                        tokio::time::sleep(remote_poll_interval()).await;
                        continue;
                    }
                }
            } else {
                match ensure_remote_started(store, owner_id, runner, &remote).await {
                    Ok(handle) => remote.handle = handle,
                    Err(error) => {
                        if permanent_remote_start_error(&error) {
                            finish_remote_run(
                                store,
                                owner_id,
                                &remote,
                                wisp_store::RunStatus::Failed,
                                Some(69),
                            )
                            .await?;
                            return Ok(());
                        }
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
                        tokio::time::sleep(remote_poll_interval()).await;
                        continue;
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
                    store
                        .record_run_poll_owned(&remote.run_id, owner_id, None, None, Some(&error))
                        .await
                        .map_err(|e| e.to_string())?;
                }
            }
        } else {
            match poll_remote(runner, &remote.handle).await {
                Ok(poll) => {
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
                    // Transport failures are transient. The persisted handle lets a
                    // later poll or a restarted app reattach without duplicating work.
                    store
                        .record_run_poll_owned(&remote.run_id, owner_id, None, None, Some(&error))
                        .await
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        tokio::time::sleep(remote_poll_interval()).await;
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
