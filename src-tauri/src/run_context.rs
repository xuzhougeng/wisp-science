use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::Mutex;
use wisp_llm::ToolSchema;
use wisp_tools::{Tool, ToolEnv, ToolResult};

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

fn resolve_input_paths(root: &Path, refs: &[String]) -> Result<Vec<PathBuf>, String> {
    if refs.is_empty() {
        return Ok(Vec::new());
    }
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|e| format!("cannot resolve project root {}: {e}", root.display()))?;
    let mut names = HashSet::new();
    refs.iter()
        .map(|value| {
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
                return Err(format!("SSH input must be project-relative: {value}"));
            }
            let path = std::fs::canonicalize(canonical_root.join(relative))
                .map_err(|e| format!("cannot resolve SSH input {value}: {e}"))?;
            if !path.starts_with(&canonical_root) || !path.is_file() {
                return Err(format!("SSH input is not a project file: {value}"));
            }
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("SSH input filename is not UTF-8: {value}"))?;
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            {
                return Err(format!(
                    "SSH input filename must use letters, numbers, '.', '_' or '-': {name}"
                ));
            }
            if !names.insert(name.to_string()) {
                return Err(format!("SSH inputs contain duplicate filename: {name}"));
            }
            Ok(path)
        })
        .collect()
}

fn ssh_script_command(
    connection: &crate::ssh_hosts::SshConnection,
    label: &str,
    payload: String,
) -> Result<RunCommand, String> {
    let mut args = connection.ssh_args()?;
    args.push("sh -s".into());
    Ok(RunCommand {
        context_id: format!("ssh:{}", connection.alias),
        program: "ssh".into(),
        args,
        script: label.into(),
        cwd: None,
        stdin: Some(payload),
    })
}

fn checked_output(
    label: &str,
    output: Result<RunCommandOutput, String>,
) -> Result<RunCommandOutput, String> {
    let output = output?;
    if output.exit_code == 0 {
        Ok(output)
    } else {
        let detail = if output.stderr.trim().is_empty() {
            output.stdout.trim()
        } else {
            output.stderr.trim()
        };
        Err(format!(
            "{label} failed with exit {}: {detail}",
            output.exit_code
        ))
    }
}

enum PrepareRemote {
    Prepared,
    Existing(RemoteRunHandle),
}

fn remote_parts(
    handle: &RemoteRunHandle,
) -> (
    &crate::ssh_hosts::SshConnection,
    &str,
    &str,
    Option<i64>,
    Option<u64>,
) {
    match handle {
        RemoteRunHandle::SshDirect {
            connection,
            workdir,
            token,
            pgid,
            start_time,
        } => (connection, workdir, token, *pgid, *start_time),
    }
}

fn handle_from_ack(handle: &RemoteRunHandle, stdout: &str) -> Result<RemoteRunHandle, String> {
    const PREFIX: &str = "__WISP_HANDLE__:";
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix(PREFIX))
        .ok_or_else(|| "SSH launcher did not return a remote handle".to_string())?;
    let mut fields = line.trim().split(':');
    let ack_token = fields.next().unwrap_or_default();
    let pgid = fields
        .next()
        .ok_or_else(|| "SSH launcher omitted PGID".to_string())?
        .parse::<i64>()
        .map_err(|_| "SSH launcher returned an invalid PGID".to_string())?;
    let start_time = fields
        .next()
        .ok_or_else(|| "SSH launcher omitted process start time".to_string())?
        .parse::<u64>()
        .map_err(|_| "SSH launcher returned an invalid process start time".to_string())?;
    if fields.next().is_some() || pgid <= 1 {
        return Err("SSH launcher returned a malformed remote handle".into());
    }
    match handle {
        RemoteRunHandle::SshDirect {
            connection,
            workdir,
            token,
            ..
        } if token == ack_token => Ok(RemoteRunHandle::SshDirect {
            connection: connection.clone(),
            workdir: workdir.clone(),
            token: token.clone(),
            pgid: Some(pgid),
            start_time: Some(start_time),
        }),
        _ => Err("SSH launcher token does not match this Run".into()),
    }
}

fn command_delimiter(token: &str, command: &str) -> String {
    let mut delimiter = format!("__WISP_COMMAND_{}__", token.replace('-', "_"));
    while command.lines().any(|line| line == delimiter) {
        delimiter.push('X');
    }
    delimiter
}

fn prepare_payload(remote: &RemoteRun) -> String {
    let (_, workdir, token, _, _) = remote_parts(&remote.handle);
    let delimiter = command_delimiter(token, &remote.command);
    format!(
        r#"set -eu
umask 077
workdir="$HOME/{workdir}"
mkdir -p "$workdir"
mkdir -p "$workdir/inputs"
if [ -f "$workdir/token" ]; then
  [ "$(cat "$workdir/token")" = "{token}" ] || {{ echo 'wisp token mismatch' >&2; exit 73; }}
else
  printf '%s\n' '{token}' > "$workdir/token.tmp"
  mv "$workdir/token.tmp" "$workdir/token"
fi
if [ -f "$workdir/_submitted" ]; then
  printf '__WISP_HANDLE__:'
  cat "$workdir/_submitted"
  exit 0
fi
cat > "$workdir/command.sh" <<'{delimiter}'
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/inputs"
{command}
{delimiter}
cat > "$workdir/supervisor.sh" <<'__WISP_SUPERVISOR__'
#!/bin/sh
set +e
umask 077
cd "$(dirname "$0")" || exit 125
write_state() {{
  path=$1
  value=$2
  tmp="$path.tmp.$$"
  printf '%s\n' "$value" > "$tmp" && mv "$tmp" "$path"
}}
if ! command -v setsid >/dev/null 2>&1 || ! command -v timeout >/dev/null 2>&1 || ! command -v bash >/dev/null 2>&1; then
  write_state _status 'lost:ssh direct Run requires setsid, timeout, and bash'
  exit 69
fi
rm -f _command_exit
setsid timeout -k 10 {timeout_secs} sh -c 'bash -l "$1"; rc=$?; tmp="$2.tmp.$$"; printf "%s\\n" "$rc" > "$tmp" && mv "$tmp" "$2"; exit "$rc"' sh "$PWD/command.sh" "$PWD/_command_exit" >stdout.log 2>stderr.log &
pgid=$!
i=0
start_time=''
while [ "$i" -lt 5 ]; do
  start_time=$(awk '{{print $22}}' "/proc/$pgid/stat" 2>/dev/null || true)
  process_group=$(awk '{{print $5}}' "/proc/$pgid/stat" 2>/dev/null || true)
  if [ -n "$start_time" ] && [ "$process_group" = "$pgid" ]; then
    break
  fi
  sleep 1
  i=$((i + 1))
done
if [ -z "$start_time" ] || [ "$process_group" != "$pgid" ]; then
  write_state _status 'lost:command process group did not start'
  exit 69
fi
write_state _submitted '{token}:'"$pgid:$start_time"
write_state _status running
wait "$pgid"
rc=$?
if [ -f _cancel_requested ]; then
  write_state _status cancelled
elif [ -f _command_exit ]; then
  command_rc=$(cat _command_exit 2>/dev/null || printf '%s' "$rc")
  write_state _status "done:$command_rc"
elif [ "$rc" = 124 ] || [ "$rc" = 137 ]; then
  write_state _status 'timed_out:124'
else
  write_state _status "done:$rc"
fi
exit "$rc"
__WISP_SUPERVISOR__
chmod 700 "$workdir/command.sh" "$workdir/supervisor.sh"
printf '__WISP_PREPARED__\n'
"#,
        command = remote.command,
        timeout_secs = remote.timeout.as_secs(),
    )
}

async fn prepare_remote(
    runner: &dyn RunCommandRunner,
    remote: &RemoteRun,
) -> Result<PrepareRemote, String> {
    let (connection, _, _, _, _) = remote_parts(&remote.handle);
    let output = checked_output(
        "SSH prepare",
        runner
            .run(
                ssh_script_command(connection, "prepare SSH Run", prepare_payload(remote))?,
                REMOTE_RPC_TIMEOUT,
            )
            .await,
    )?;
    if output
        .stdout
        .lines()
        .any(|line| line == "__WISP_PREPARED__")
    {
        Ok(PrepareRemote::Prepared)
    } else {
        Ok(PrepareRemote::Existing(handle_from_ack(
            &remote.handle,
            &output.stdout,
        )?))
    }
}

async fn stage_remote_inputs(
    runner: &dyn RunCommandRunner,
    remote: &RemoteRun,
) -> Result<(), String> {
    if remote.input_refs.is_empty() {
        return Ok(());
    }
    let root = remote
        .harvest_root
        .as_deref()
        .ok_or_else(|| "SSH input staging requires its project workspace".to_string())?;
    let input_paths = resolve_input_paths(root, &remote.input_refs)?;
    let (connection, workdir, _, _, _) = remote_parts(&remote.handle);
    let mut args = connection.scp_option_args()?;
    args.extend(
        input_paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned()),
    );
    args.push(format!("{}:{workdir}/inputs/", connection.target()?));
    checked_output(
        "SSH input staging",
        runner
            .run(
                RunCommand {
                    context_id: format!("ssh:{}", connection.alias),
                    program: "scp".into(),
                    args,
                    script: format!("stage {} input file(s)", input_paths.len()),
                    cwd: remote.harvest_root.clone(),
                    stdin: None,
                },
                Duration::from_secs(300),
            )
            .await,
    )?;
    Ok(())
}

fn launch_payload(handle: &RemoteRunHandle) -> String {
    let (_, workdir, token, _, _) = remote_parts(handle);
    format!(
        r#"set -eu
workdir="$HOME/{workdir}"
[ -f "$workdir/token" ] && [ "$(cat "$workdir/token")" = "{token}" ] || {{ echo 'wisp token mismatch' >&2; exit 73; }}
lock="$workdir/_launch_lock"
if [ -d "$lock" ] && [ ! -f "$workdir/_submitted" ]; then
  owner=$(cat "$lock/owner" 2>/dev/null || true)
  lock_pid=${{owner%%:*}}
  lock_start=${{owner#*:}}
  current=$(awk '{{print $22}}' "/proc/$lock_pid/stat" 2>/dev/null || true)
  if [ -z "$lock_pid" ] || [ "$current" != "$lock_start" ]; then
    rm -f "$lock/owner"
    rmdir "$lock" 2>/dev/null || true
  fi
fi
if [ ! -f "$workdir/_submitted" ] && mkdir "$lock" 2>/dev/null; then
  trap 'rm -f "$lock/owner"; rmdir "$lock" 2>/dev/null || true' EXIT HUP INT TERM
  lock_start=$(awk '{{print $22}}' "/proc/$$/stat" 2>/dev/null || true)
  printf '%s:%s\n' "$$" "$lock_start" > "$lock/owner"
  command -v setsid >/dev/null 2>&1 || {{ echo 'SSH direct Runs require setsid' >&2; exit 69; }}
  command -v timeout >/dev/null 2>&1 || {{ echo 'SSH direct Runs require timeout' >&2; exit 69; }}
  command -v bash >/dev/null 2>&1 || {{ echo 'SSH direct Runs require bash' >&2; exit 69; }}
  nohup setsid sh "$workdir/supervisor.sh" </dev/null >/dev/null 2>&1 &
fi
if [ ! -f "$workdir/_submitted" ]; then
  i=0
  while [ ! -f "$workdir/_submitted" ] && [ "$i" -lt 10 ]; do
    sleep 1
    i=$((i + 1))
  done
fi
[ -f "$workdir/_submitted" ] || {{ echo 'remote supervisor did not acknowledge launch' >&2; exit 70; }}
printf '__WISP_HANDLE__:'
cat "$workdir/_submitted"
"#,
    )
}

async fn launch_remote(
    runner: &dyn RunCommandRunner,
    handle: &RemoteRunHandle,
) -> Result<RemoteRunHandle, String> {
    let (connection, _, _, _, _) = remote_parts(handle);
    let output = checked_output(
        "SSH launch",
        runner
            .run(
                ssh_script_command(connection, "launch SSH Run", launch_payload(handle))?,
                REMOTE_RPC_TIMEOUT,
            )
            .await,
    )?;
    handle_from_ack(handle, &output.stdout)
}

async fn ensure_remote_started(
    store: &wisp_store::Store,
    owner_id: &str,
    runner: &dyn RunCommandRunner,
    remote: &RemoteRun,
) -> Result<RemoteRunHandle, String> {
    if remote.handle.is_confirmed() {
        return Ok(remote.handle.clone());
    }
    match prepare_remote(runner, remote).await? {
        PrepareRemote::Existing(handle) => Ok(handle),
        PrepareRemote::Prepared => {
            stage_remote_inputs(runner, remote).await?;
            let run = store
                .get_run(&remote.run_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Run not found: {}", remote.run_id))?;
            if run.status == wisp_store::RunStatus::Cancelling {
                return Err("SSH Run was cancelled before launch".into());
            }
            if !store
                .renew_run_lifecycle(&remote.run_id, owner_id, REMOTE_START_LEASE_SECS)
                .await
                .map_err(|e| e.to_string())?
            {
                return Err("SSH lifecycle lease expired before launch".into());
            }
            launch_remote(runner, &remote.handle).await
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RemotePollState {
    Running,
    Finished(i64),
    TimedOut(i64),
    Cancelled,
    Lost(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemotePoll {
    state: RemotePollState,
    stdout: String,
    stderr: String,
}

fn poll_payload(handle: &RemoteRunHandle) -> Result<String, String> {
    let (_, workdir, token, pgid, start_time) = remote_parts(handle);
    let (pgid, start_time) = pgid
        .zip(start_time)
        .ok_or_else(|| "SSH Run handle has not been confirmed".to_string())?;
    Ok(format!(
        r#"set -eu
workdir="$HOME/{workdir}"
state='lost:control directory missing'
same_identity() {{
  current=$(awk '{{print $22}}' "/proc/{pgid}/stat" 2>/dev/null || true)
  group=$(awk '{{print $5}}' "/proc/{pgid}/stat" 2>/dev/null || true)
  [ "$current" = "{start_time}" ] && [ "$group" = "{pgid}" ] && kill -0 "-{pgid}" 2>/dev/null
}}
read_status() {{
  status=$(cat "$workdir/_status" 2>/dev/null || true)
  case "$status" in
    done:*) state="finished:${{status#done:}}"; return 0 ;;
    timed_out:*) state="$status"; return 0 ;;
    cancelled) state='cancelled'; return 0 ;;
    lost:*) state="$status"; return 0 ;;
  esac
  return 1
}}
if [ -f "$workdir/token" ] && [ "$(cat "$workdir/token")" = "{token}" ]; then
  if ! read_status; then
    if same_identity; then
      state='running'
    else
      # A supervisor writes _status immediately after its child exits. Re-read
      # once before declaring the process lost at that boundary.
      sleep 1
      if read_status; then
        :
      elif same_identity; then
        state='running'
      else
        state='lost:remote process handle no longer exists'
      fi
    fi
  fi
fi
printf '__WISP_RUN_STATUS__:%s\n' "$state"
printf '__WISP_STDOUT__\n'
tail -c 4000 "$workdir/stdout.log" 2>/dev/null || true
printf '\n__WISP_STDERR__\n'
tail -c 4000 "$workdir/stderr.log" 2>/dev/null || true
"#,
    ))
}

fn parse_remote_poll(stdout: &str) -> Result<RemotePoll, String> {
    const STATUS: &str = "__WISP_RUN_STATUS__:";
    const STDOUT: &str = "__WISP_STDOUT__\n";
    const STDERR: &str = "\n__WISP_STDERR__\n";
    let start = stdout
        .find(STATUS)
        .ok_or_else(|| "SSH poll response omitted status".to_string())?;
    let after = &stdout[start + STATUS.len()..];
    let (status, body) = after
        .split_once('\n')
        .ok_or_else(|| "SSH poll response has a malformed status".to_string())?;
    let body = body
        .strip_prefix(STDOUT)
        .ok_or_else(|| "SSH poll response omitted stdout marker".to_string())?;
    let (stdout_tail, stderr_tail) = body
        .split_once(STDERR)
        .ok_or_else(|| "SSH poll response omitted stderr marker".to_string())?;
    let state = if status == "running" {
        RemotePollState::Running
    } else if status == "cancelled" {
        RemotePollState::Cancelled
    } else if let Some(code) = status.strip_prefix("finished:") {
        RemotePollState::Finished(
            code.parse::<i64>()
                .map_err(|_| "SSH poll returned an invalid exit code".to_string())?,
        )
    } else if let Some(code) = status.strip_prefix("timed_out:") {
        RemotePollState::TimedOut(
            code.parse::<i64>()
                .map_err(|_| "SSH poll returned an invalid timeout code".to_string())?,
        )
    } else if let Some(reason) = status.strip_prefix("lost:") {
        RemotePollState::Lost(reason.into())
    } else {
        return Err(format!("SSH poll returned unknown state: {status}"));
    };
    Ok(RemotePoll {
        state,
        stdout: stdout_tail.trim_end_matches('\n').into(),
        stderr: stderr_tail.trim_end_matches('\n').into(),
    })
}

async fn poll_remote(
    runner: &dyn RunCommandRunner,
    handle: &RemoteRunHandle,
) -> Result<RemotePoll, String> {
    let (connection, _, _, _, _) = remote_parts(handle);
    let output = checked_output(
        "SSH poll",
        runner
            .run(
                ssh_script_command(connection, "poll SSH Run", poll_payload(handle)?)?,
                REMOTE_RPC_TIMEOUT,
            )
            .await,
    )?;
    parse_remote_poll(&output.stdout)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RemoteCancel {
    Cancelled,
    Finished(i64),
    TimedOut(i64),
    Lost(String),
}

fn cancel_payload(handle: &RemoteRunHandle) -> Result<String, String> {
    let (_, workdir, token, pgid, start_time) = remote_parts(handle);
    let (pgid, start_time) = pgid
        .zip(start_time)
        .ok_or_else(|| "SSH Run handle has not been confirmed".to_string())?;
    Ok(format!(
        r#"set -eu
workdir="$HOME/{workdir}"
same_identity() {{
  current=$(awk '{{print $22}}' "/proc/{pgid}/stat" 2>/dev/null || true)
  group=$(awk '{{print $5}}' "/proc/{pgid}/stat" 2>/dev/null || true)
  [ "$current" = "{start_time}" ] && [ "$group" = "{pgid}" ] && kill -0 "-{pgid}" 2>/dev/null
}}
terminal_status() {{
  status=$(cat "$workdir/_status" 2>/dev/null || true)
  case "$status" in
    done:*) printf '__WISP_CANCEL__:finished:%s\n' "${{status#done:}}"; return 0 ;;
    timed_out:*) printf '__WISP_CANCEL__:timed_out:%s\n' "${{status#timed_out:}}"; return 0 ;;
    cancelled) printf '__WISP_CANCEL__:cancelled\n'; return 0 ;;
  esac
  return 1
}}
if [ ! -f "$workdir/token" ] || [ "$(cat "$workdir/token")" != "{token}" ]; then
  printf '__WISP_CANCEL__:lost:token mismatch\n'
  exit 0
fi
terminal_status && exit 0 || true
if ! same_identity; then
  sleep 1
  terminal_status && exit 0 || true
  printf '__WISP_CANCEL__:retry:process identity changed\n'
  exit 0
fi
if ! kill -TERM "-{pgid}" 2>/dev/null; then
  printf '__WISP_CANCEL__:retry:TERM was not confirmed\n'
  exit 0
fi
tmp="$workdir/_cancel_requested.tmp.$$"
printf 'requested\n' > "$tmp" && mv "$tmp" "$workdir/_cancel_requested"
i=0
while [ "$i" -lt 10 ]; do
  terminal_status && exit 0 || true
  kill -0 "-{pgid}" 2>/dev/null || break
  sleep 1
  i=$((i + 1))
done
if same_identity; then
  kill -KILL "-{pgid}" 2>/dev/null || true
fi
i=0
while kill -0 "-{pgid}" 2>/dev/null && [ "$i" -lt 5 ]; do
  sleep 1
  i=$((i + 1))
done
terminal_status && exit 0 || true
if kill -0 "-{pgid}" 2>/dev/null; then
  printf '__WISP_CANCEL__:retry:process group survived cancellation\n'
  exit 0
fi
tmp="$workdir/_status.tmp.$$"
printf 'cancelled\n' > "$tmp" && mv "$tmp" "$workdir/_status"
printf '__WISP_CANCEL__:cancelled\n'
"#,
    ))
}

fn parse_remote_cancel(stdout: &str) -> Result<RemoteCancel, String> {
    const PREFIX: &str = "__WISP_CANCEL__:";
    let value = stdout
        .lines()
        .find_map(|line| line.strip_prefix(PREFIX))
        .ok_or_else(|| "SSH cancel response omitted status".to_string())?;
    if value == "cancelled" {
        Ok(RemoteCancel::Cancelled)
    } else if let Some(code) = value.strip_prefix("finished:") {
        Ok(RemoteCancel::Finished(code.parse::<i64>().map_err(
            |_| "SSH cancel returned an invalid exit code".to_string(),
        )?))
    } else if let Some(code) = value.strip_prefix("timed_out:") {
        Ok(RemoteCancel::TimedOut(code.parse::<i64>().map_err(
            |_| "SSH cancel returned an invalid timeout code".to_string(),
        )?))
    } else if let Some(reason) = value.strip_prefix("lost:") {
        Ok(RemoteCancel::Lost(reason.into()))
    } else {
        Err(format!("SSH cancel returned unknown state: {value}"))
    }
}

async fn cancel_remote(
    runner: &dyn RunCommandRunner,
    handle: &RemoteRunHandle,
) -> Result<RemoteCancel, String> {
    let (connection, _, _, _, _) = remote_parts(handle);
    let output = checked_output(
        "SSH cancel",
        runner
            .run(
                ssh_script_command(connection, "cancel SSH Run", cancel_payload(handle)?)?,
                REMOTE_RPC_TIMEOUT,
            )
            .await,
    )?;
    parse_remote_cancel(&output.stdout)
}

fn remote_terminal_status(exit_code: i64) -> wisp_store::RunStatus {
    match exit_code {
        0 => wisp_store::RunStatus::Succeeded,
        _ => wisp_store::RunStatus::Failed,
    }
}

fn remote_poll_interval() -> Duration {
    if cfg!(test) {
        Duration::from_millis(10)
    } else {
        Duration::from_secs(5)
    }
}

fn permanent_remote_start_error(error: &str) -> bool {
    error.contains("requires setsid")
        || error.contains("requires timeout")
        || error.contains("requires bash")
        || error.contains("process group did not start")
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

pub struct RunInContextTool {
    store: wisp_store::Store,
    manager: RunManager,
    project_id: String,
    frame_id: Option<String>,
}

impl RunInContextTool {
    pub fn new(
        store: wisp_store::Store,
        manager: RunManager,
        project_id: String,
        frame_id: Option<String>,
    ) -> Self {
        Self {
            store,
            manager,
            project_id,
            frame_id,
        }
    }
}

#[async_trait::async_trait]
impl Tool for RunInContextTool {
    fn name(&self) -> &str {
        "run_in_context"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "run_in_context",
            "Submit a persisted background Run in an execution context (`local`, `ssh:<alias>`, or `wsl:<distro>`). SSH Runs detach on the server and return after launch; use get_run or the Runs panel later instead of shell sleep/ps polling.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "context_id": { "type": "string", "description": "Execution context id, e.g. local, ssh:gpu, wsl:Ubuntu" },
                    "command": { "type": "string", "description": "Command to execute in that context" },
                    "title": { "type": "string", "description": "Short run title" },
                    "timeout_secs": { "type": "integer", "description": "Job wall timeout. SSH: 1s..7d (default 4h); local/WSL: 1s..300s" },
                    "input_paths": {
                        "type": "array",
                        "description": "Optional project-relative files staged flat into an SSH Run workdir",
                        "items": { "type": "string" }
                    },
                    "output_specs": {
                        "type": "array",
                        "description": "Optional output specs. SSH direct currently accepts explicit ssh:// references only",
                        "items": {
                            "type": "object",
                            "properties": {
                                "glob": { "type": "string" },
                                "kind": { "type": "string" },
                                "residency": { "type": "string", "enum": ["local", "remote", "auto"] },
                                "max_file_mb": { "type": "integer" },
                                "max_total_mb": { "type": "integer" }
                            },
                            "required": ["glob", "kind", "residency"]
                        }
                    }
                },
                "required": ["context_id", "command"]
            }),
        )
    }

    fn preview(&self, args: &serde_json::Value) -> String {
        let context = args
            .get("context_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
        format!("{context}: {command}")
    }

    async fn run(&self, args: &serde_json::Value, env: &dyn ToolEnv) -> ToolResult {
        let request: SubmitRunRequest = match serde_json::from_value(args.clone()) {
            Ok(req) => req,
            Err(e) => return ToolResult::fail(format!("run_in_context args error: {e}")),
        };
        if !env.danger_auto_approve() {
            if let Some(danger) = wisp_tools::safety::check_command_safety(&request.command) {
                let msg = format!(
                    "Dangerous command detected in run_in_context ({}): {}",
                    danger.label(),
                    request.command
                );
                if !env.confirm(&msg).await {
                    return ToolResult::fail("error: User denied action");
                }
            }
        }
        match self
            .manager
            .submit(
                self.store.clone(),
                self.project_id.clone(),
                self.frame_id.clone(),
                request,
                Some(env.project_root().to_path_buf()),
            )
            .await
        {
            Ok(res) => ToolResult::ok(serde_json::to_string(&res).unwrap_or_default()),
            Err(e) => ToolResult::fail(format!("run_in_context error: {e}")),
        }
    }
}

pub struct GetRunTool {
    store: wisp_store::Store,
    project_id: String,
}

impl GetRunTool {
    pub fn new(store: wisp_store::Store, project_id: String) -> Self {
        Self { store, project_id }
    }
}

#[async_trait::async_trait]
impl Tool for GetRunTool {
    fn name(&self) -> &str {
        "get_run"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "get_run",
            "Read the latest persisted status, output tails, remote workdir, and SSH poll health for a Run. This does not wait for completion.",
            serde_json::json!({
                "type": "object",
                "properties": { "run_id": { "type": "string" } },
                "required": ["run_id"]
            }),
        )
    }

    fn preview(&self, args: &serde_json::Value) -> String {
        args.get("run_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .into()
    }

    async fn run(&self, args: &serde_json::Value, _env: &dyn ToolEnv) -> ToolResult {
        let Some(run_id) = args.get("run_id").and_then(|value| value.as_str()) else {
            return ToolResult::fail("get_run requires run_id");
        };
        match self.store.get_run(run_id).await {
            Ok(Some(run)) if run.project_id == self.project_id => {
                ToolResult::ok(serde_json::to_string(&run).unwrap_or_default())
            }
            Ok(Some(_)) => ToolResult::fail("Run does not belong to this project"),
            Ok(None) => ToolResult::fail("Run not found"),
            Err(error) => ToolResult::fail(format!("get_run error: {error}")),
        }
    }
}

pub struct CancelRunTool {
    store: wisp_store::Store,
    manager: RunManager,
    project_id: String,
}

impl CancelRunTool {
    pub fn new(store: wisp_store::Store, manager: RunManager, project_id: String) -> Self {
        Self {
            store,
            manager,
            project_id,
        }
    }
}

#[async_trait::async_trait]
impl Tool for CancelRunTool {
    fn name(&self) -> &str {
        "cancel_run"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "cancel_run",
            "Request cancellation of a submitted or running Run. SSH Runs remain `cancelling` until the remote process group confirms termination.",
            serde_json::json!({
                "type": "object",
                "properties": { "run_id": { "type": "string" } },
                "required": ["run_id"]
            }),
        )
    }

    fn preview(&self, args: &serde_json::Value) -> String {
        args.get("run_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .into()
    }

    async fn run(&self, args: &serde_json::Value, _env: &dyn ToolEnv) -> ToolResult {
        let Some(run_id) = args.get("run_id").and_then(|value| value.as_str()) else {
            return ToolResult::fail("cancel_run requires run_id");
        };
        match self.store.get_run(run_id).await {
            Ok(Some(run)) if run.project_id == self.project_id => {}
            Ok(Some(_)) => return ToolResult::fail("Run does not belong to this project"),
            Ok(None) => return ToolResult::fail("Run not found"),
            Err(error) => return ToolResult::fail(format!("cancel_run error: {error}")),
        }
        match self.manager.cancel(&self.store, run_id).await {
            Ok(()) => match self.store.get_run(run_id).await {
                Ok(Some(run)) => ToolResult::ok(serde_json::to_string(&run).unwrap_or_default()),
                Ok(None) => ToolResult::fail("Run disappeared after cancellation request"),
                Err(error) => ToolResult::fail(format!("cancel_run error: {error}")),
            },
            Err(error) => ToolResult::fail(format!("cancel_run error: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    #[tokio::test]
    async fn run_in_context_preview_keeps_long_commands_intact() {
        use wisp_tools::Tool;
        let tmp =
            std::env::temp_dir().join(format!("wisp_run_preview_{}.sqlite", uuid::Uuid::new_v4()));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        let tool = RunInContextTool::new(store, RunManager::new(), "p".into(), None);
        let command = format!(
            "grep -in snakemake {} {}",
            "/data/xzg_data/2026-07-07-Cerichardii-rnaseq/omics-pipelines/rnaseq/README.md",
            "/data/xzg_data/2026-07-07-Cerichardii-rnaseq/omics-pipelines/rnaseq/Snakefile"
        );
        assert!(
            command.len() > 140,
            "premise: command longer than old 140-char cap"
        );
        let preview = tool.preview(&serde_json::json!({
            "context_id": "ssh:CPU3",
            "command": command.clone(),
        }));
        assert_eq!(preview, format!("ssh:CPU3: {command}"));
        let _ = std::fs::remove_file(tmp);
    }

    #[test]
    fn builds_commands_for_local_ssh_and_wsl() {
        let local = wisp_store::ExecutionContext::new("local", "Local").unwrap();
        let ssh = wisp_store::ExecutionContext::new("ssh:gpu-box", "GPU").unwrap();
        let wsl = wisp_store::ExecutionContext::new("wsl:Ubuntu-22.04", "Ubuntu").unwrap();

        let local_cmd = build_run_command(&local, "echo hi", Some(PathBuf::from("/tmp")));
        assert_eq!(local_cmd.script, "echo hi");
        assert_eq!(local_cmd.cwd.as_deref(), Some(std::path::Path::new("/tmp")));
        assert!(!local_cmd.program.is_empty());

        let ssh_cmd = build_run_command(&ssh, "echo hi", None);
        assert_eq!(ssh_cmd.program, "ssh");
        assert_eq!(ssh_cmd.args[0], "gpu-box");

        let wsl_cmd = build_run_command(&wsl, "echo hi", None);
        assert_eq!(wsl_cmd.program, "wsl.exe");
        assert!(wsl_cmd.args.contains(&"-d".to_string()));
        assert!(wsl_cmd.args.contains(&"Ubuntu-22.04".to_string()));
    }

    #[tokio::test]
    async fn submit_run_records_success() {
        let tmp =
            std::env::temp_dir().join(format!("wisp_submit_run_{}.sqlite", uuid::Uuid::new_v4()));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        store.create_project("p", "proj", "").await.unwrap();
        store
            .upsert_execution_context(&wisp_store::ExecutionContext::new("local", "Local").unwrap())
            .await
            .unwrap();
        let runner = FakeRunRunner {
            output: Ok(RunCommandOutput {
                exit_code: 0,
                stdout: "hello\n".into(),
                stderr: String::new(),
            }),
        };

        let res = submit_run_with_runner(
            &store,
            "p",
            None,
            SubmitRunRequest {
                context_id: "local".into(),
                command: "echo hello".into(),
                title: Some("Hello".into()),
                timeout_secs: Some(5),
                input_paths: None,
                output_specs: None,
            },
            &runner,
            None,
        )
        .await
        .unwrap();

        assert_eq!(res.status, wisp_store::RunStatus::Succeeded);
        assert_eq!(res.exit_code, Some(0));
        assert_eq!(res.stdout_tail.as_deref(), Some("hello\n"));
        let run = store.get_run(&res.run_id).await.unwrap().unwrap();
        assert_eq!(run.context_id, "local");
        assert_eq!(run.command.as_deref(), Some("echo hello"));
        assert_eq!(run.title, "Hello");
        assert_eq!(run.status, wisp_store::RunStatus::Succeeded);

        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn submit_run_records_failure() {
        let tmp = std::env::temp_dir().join(format!(
            "wisp_submit_run_fail_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        store.create_project("p", "proj", "").await.unwrap();
        store
            .upsert_execution_context(&wisp_store::ExecutionContext::new("local", "Local").unwrap())
            .await
            .unwrap();
        let runner = FakeRunRunner {
            output: Err("timed out".into()),
        };

        let res = submit_run_with_runner(
            &store,
            "p",
            None,
            SubmitRunRequest {
                context_id: "local".into(),
                command: "sleep 10".into(),
                title: None,
                timeout_secs: Some(1),
                input_paths: None,
                output_specs: None,
            },
            &runner,
            None,
        )
        .await
        .unwrap();

        assert_eq!(res.status, wisp_store::RunStatus::Failed);
        assert_eq!(res.exit_code, Some(-1));
        assert_eq!(res.stderr_tail.as_deref(), Some("timed out"));
        let run = store.get_run(&res.run_id).await.unwrap().unwrap();
        assert_eq!(run.status, wisp_store::RunStatus::Failed);
        assert_eq!(run.stderr_tail.as_deref(), Some("timed out"));

        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn submit_run_harvests_output_specs_on_success() {
        let tmp =
            std::env::temp_dir().join(format!("wisp_submit_run_harvest_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(tmp.join("results")).unwrap();
        std::fs::write(tmp.join("results/out.tsv"), b"x\ty\n").unwrap();
        let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
            .await
            .unwrap();
        store.create_project("p", "proj", "").await.unwrap();
        store.create_frame("f", "p", "OPERON", "m").await.unwrap();
        store
            .upsert_execution_context(&wisp_store::ExecutionContext::new("local", "Local").unwrap())
            .await
            .unwrap();
        let runner = FakeRunRunner {
            output: Ok(RunCommandOutput {
                exit_code: 0,
                stdout: "done".into(),
                stderr: String::new(),
            }),
        };

        let res = submit_run_with_runner(
            &store,
            "p",
            Some("f"),
            SubmitRunRequest {
                context_id: "local".into(),
                command: "make outputs".into(),
                title: None,
                timeout_secs: Some(5),
                input_paths: None,
                output_specs: Some(vec![crate::harvest::OutputSpec {
                    glob: "results/*.tsv".into(),
                    kind: "table".into(),
                    residency: crate::harvest::OutputResidency::Auto,
                    max_file_mb: Some(1),
                    max_total_mb: Some(1),
                }]),
            },
            &runner,
            Some(tmp.clone()),
        )
        .await
        .unwrap();

        let links = store.list_run_artifacts(&res.run_id).await.unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].1, "table");
        assert_eq!(store.list_artifacts("f").await.unwrap().len(), 1);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn background_run_can_be_cancelled_without_waiting_for_the_command() {
        let tmp = std::env::temp_dir().join(format!(
            "wisp_background_run_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        store.create_project("p", "proj", "").await.unwrap();
        let manager = RunManager::with_runner(Arc::new(PendingRunRunner));

        let submitted = manager
            .submit(
                store.clone(),
                "p".into(),
                None,
                SubmitRunRequest {
                    context_id: "local".into(),
                    command: "long-running-analysis".into(),
                    title: None,
                    timeout_secs: Some(60),
                    input_paths: None,
                    output_specs: None,
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(submitted.status, wisp_store::RunStatus::Submitted);

        manager.cancel(&store, &submitted.run_id).await.unwrap();
        let run = store.get_run(&submitted.run_id).await.unwrap().unwrap();
        assert_eq!(run.status, wisp_store::RunStatus::Cancelled);

        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn ssh_run_detaches_persists_handle_and_finishes_from_poller() {
        let tmp = std::env::temp_dir().join(format!("wisp_ssh_lifecycle_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
            .await
            .unwrap();
        store
            .create_project("p", "proj", &tmp.to_string_lossy())
            .await
            .unwrap();
        store
            .upsert_execution_context(&wisp_store::ExecutionContext::new("ssh:gpu", "GPU").unwrap())
            .await
            .unwrap();
        let runner = Arc::new(ScriptedRunRunner::new(vec![
            ok_output("__WISP_PREPARED__\n"),
            ok_output("__WISP_HANDLE__:token-will-be-replaced"),
        ]));
        let manager = RunManager::with_runner(runner.clone());

        // The launch ACK contains a per-run token, so let the scripted runner
        // synthesize it from the prepare payload instead of hard-coding it.
        runner
            .synthesize_launch_ack
            .store(true, std::sync::atomic::Ordering::SeqCst);
        runner.push(ok_output(&poll_response("finished:0", "complete", "")));
        let command = "printf '%s\\n' '$HOME' && printf '%s\\n' '$(date)'";
        let submitted = manager
            .submit(
                store.clone(),
                "p".into(),
                None,
                SubmitRunRequest {
                    context_id: "ssh:gpu".into(),
                    command: command.into(),
                    title: Some("Remote analysis".into()),
                    timeout_secs: Some(3600),
                    input_paths: None,
                    output_specs: None,
                },
                Some(tmp.clone()),
            )
            .await
            .unwrap();

        assert!(matches!(
            submitted.status,
            wisp_store::RunStatus::Submitted | wisp_store::RunStatus::Running
        ));
        assert!(submitted
            .remote_workdir
            .as_deref()
            .unwrap()
            .starts_with("~/.wisp-science/runs/"));
        let finished = wait_for_terminal(&store, &submitted.run_id).await;
        assert_eq!(finished.status, wisp_store::RunStatus::Succeeded);
        assert_eq!(finished.exit_code, Some(0));
        assert_eq!(finished.stdout_tail.as_deref(), Some("complete"));
        assert!(finished
            .remote_handle_json
            .as_deref()
            .unwrap()
            .contains("ssh_direct"));

        let commands = runner.commands.lock().unwrap();
        assert_eq!(
            commands
                .iter()
                .filter(|command| command.program == "ssh")
                .count(),
            3
        );
        assert!(commands[0].stdin.as_deref().unwrap().contains(command));
        assert!(commands[0]
            .stdin
            .as_deref()
            .unwrap()
            .contains("setsid timeout -k 10"));
        assert!(!commands[0]
            .stdin
            .as_deref()
            .unwrap()
            .contains("else\n  bash -l"));
        assert!(!commands[1].stdin.as_deref().unwrap().contains(command));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn recovery_reattaches_ssh_after_transient_error_and_marks_local_lost() {
        let tmp = std::env::temp_dir().join(format!("wisp_ssh_recover_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
            .await
            .unwrap();
        store
            .create_project("p", "proj", &tmp.to_string_lossy())
            .await
            .unwrap();

        let mut remote =
            wisp_store::RunRecord::new("remote", "p", "ssh:gpu", "Remote", "ssh_direct");
        remote.command = Some("long-analysis".into());
        remote.timeout_secs = Some(3600);
        remote.remote_workdir = Some("~/.wisp-science/runs/remote".into());
        remote.remote_handle_json =
            Some(serde_json::to_string(&test_handle("remote", true)).unwrap());
        store.create_run(&remote).await.unwrap();
        store
            .update_run_status("remote", wisp_store::RunStatus::Running)
            .await
            .unwrap();

        let local = wisp_store::RunRecord::new("local-run", "p", "local", "Local", "command");
        store.create_run(&local).await.unwrap();
        store
            .update_run_status("local-run", wisp_store::RunStatus::Running)
            .await
            .unwrap();

        let runner = Arc::new(ScriptedRunRunner::new(vec![
            Err("temporary SSH disconnect".into()),
            ok_output(&poll_response("finished:0", "reconnected", "")),
        ]));
        let manager = RunManager::with_runner(runner);
        assert_eq!(manager.recover(&store).await.unwrap(), 1);

        let finished = wait_for_terminal(&store, "remote").await;
        assert_eq!(finished.status, wisp_store::RunStatus::Succeeded);
        assert_eq!(finished.stdout_tail.as_deref(), Some("reconnected"));
        assert!(finished.last_poll_error.is_none());
        assert_eq!(
            store.get_run("local-run").await.unwrap().unwrap().status,
            wisp_store::RunStatus::Lost
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn ssh_cancel_stays_cancelling_until_remote_group_confirms() {
        let tmp = std::env::temp_dir().join(format!("wisp_ssh_cancel_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
            .await
            .unwrap();
        store
            .create_project("p", "proj", &tmp.to_string_lossy())
            .await
            .unwrap();
        let mut run = wisp_store::RunRecord::new("remote", "p", "ssh:gpu", "Remote", "ssh_direct");
        run.command = Some("long-analysis".into());
        run.timeout_secs = Some(3600);
        run.remote_workdir = Some("~/.wisp-science/runs/remote".into());
        run.remote_handle_json = Some(serde_json::to_string(&test_handle("remote", true)).unwrap());
        store.create_run(&run).await.unwrap();
        store
            .update_run_status("remote", wisp_store::RunStatus::Running)
            .await
            .unwrap();
        let runner = Arc::new(ScriptedRunRunner::new(vec![ok_output(
            "__WISP_CANCEL__:cancelled\n",
        )]));
        let manager = RunManager::with_runner(runner.clone());

        manager.cancel(&store, "remote").await.unwrap();
        assert_eq!(
            store.get_run("remote").await.unwrap().unwrap().status,
            wisp_store::RunStatus::Cancelling
        );
        assert_eq!(
            wait_for_terminal(&store, "remote").await.status,
            wisp_store::RunStatus::Cancelled
        );
        let commands = runner.commands.lock().unwrap();
        let payload = commands[0].stdin.as_deref().unwrap();
        assert!(payload.contains("kill -TERM \"-4242\""));
        assert!(!payload.contains("kill -TERM --"));
        assert!(payload.contains("/proc/4242/stat"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn tail_preserves_utf8_boundaries() {
        let s = format!("{}{}", "a".repeat(3999), "科研");
        let out = tail(&s);
        assert!(out.starts_with('a') || out.starts_with('科'));
        assert!(out.ends_with("科研"));
    }

    #[test]
    fn remote_control_payloads_are_valid_posix_shell() {
        let remote = RemoteRun {
            run_id: "payload".into(),
            project_id: "p".into(),
            frame_id: None,
            command: "printf '%s\\n' ok".into(),
            timeout: Duration::from_secs(60),
            input_refs: Vec::new(),
            output_specs: Vec::new(),
            harvest_root: None,
            handle: test_handle("payload", true),
        };
        let scripts = [
            prepare_payload(&remote),
            launch_payload(&remote.handle),
            poll_payload(&remote.handle).unwrap(),
            cancel_payload(&remote.handle).unwrap(),
        ];
        for script in scripts {
            let mut child = std::process::Command::new("sh")
                .args(["-n", "-s"])
                .stdin(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(script.as_bytes())
                .unwrap();
            assert!(child.wait().unwrap().success(), "invalid shell payload");
        }
    }

    #[test]
    fn remote_compute_skill_uses_the_real_wisp_run_contract() {
        let skill = include_str!("../../skills/remote-compute-ssh/SKILL.md");
        for tool in ["run_in_context", "get_run", "cancel_run"] {
            assert!(skill.contains(tool), "missing {tool}");
        }
        for stale in [
            "host.compute",
            "wait_for_notification",
            "compute_details",
            "submit_job",
            "attach_job",
            "repl tool",
        ] {
            assert!(!skill.contains(stale), "stale API remains: {stale}");
        }
        assert!(skill.contains("Do not wait for completion"));
        assert!(skill.contains("Scheduler lifecycle is not implemented yet"));
    }

    struct FakeRunRunner {
        output: Result<RunCommandOutput, String>,
    }

    #[async_trait::async_trait]
    impl RunCommandRunner for FakeRunRunner {
        async fn run(
            &self,
            _command: RunCommand,
            _timeout: Duration,
        ) -> Result<RunCommandOutput, String> {
            self.output.clone()
        }
    }

    struct PendingRunRunner;

    #[async_trait::async_trait]
    impl RunCommandRunner for PendingRunRunner {
        async fn run(
            &self,
            _command: RunCommand,
            _timeout: Duration,
        ) -> Result<RunCommandOutput, String> {
            std::future::pending().await
        }
    }

    struct ScriptedRunRunner {
        outputs: StdMutex<VecDeque<Result<RunCommandOutput, String>>>,
        commands: StdMutex<Vec<RunCommand>>,
        synthesize_launch_ack: std::sync::atomic::AtomicBool,
        token: StdMutex<Option<String>>,
    }

    impl ScriptedRunRunner {
        fn new(outputs: Vec<Result<RunCommandOutput, String>>) -> Self {
            Self {
                outputs: StdMutex::new(outputs.into()),
                commands: StdMutex::new(Vec::new()),
                synthesize_launch_ack: std::sync::atomic::AtomicBool::new(false),
                token: StdMutex::new(None),
            }
        }

        fn push(&self, output: Result<RunCommandOutput, String>) {
            self.outputs.lock().unwrap().push_back(output);
        }
    }

    #[async_trait::async_trait]
    impl RunCommandRunner for ScriptedRunRunner {
        async fn run(
            &self,
            command: RunCommand,
            _timeout: Duration,
        ) -> Result<RunCommandOutput, String> {
            if command.script == "prepare SSH Run" {
                if let Some(payload) = command.stdin.as_deref() {
                    let token = payload
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("  printf '%s\\n' '")?
                                .strip_suffix("' > \"$workdir/token.tmp\"")
                        })
                        .map(str::to_string);
                    *self.token.lock().unwrap() = token;
                }
            }
            self.commands.lock().unwrap().push(command.clone());
            let output = self
                .outputs
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err(format!("unexpected command: {}", command.script)))?;
            if command.script == "launch SSH Run"
                && self
                    .synthesize_launch_ack
                    .load(std::sync::atomic::Ordering::SeqCst)
            {
                let token = self.token.lock().unwrap().clone().unwrap();
                return Ok(RunCommandOutput {
                    exit_code: 0,
                    stdout: format!("__WISP_HANDLE__:{token}:4242:999\n"),
                    stderr: String::new(),
                });
            }
            Ok(output)
        }
    }

    fn ok_output(stdout: &str) -> Result<RunCommandOutput, String> {
        Ok(RunCommandOutput {
            exit_code: 0,
            stdout: stdout.into(),
            stderr: String::new(),
        })
    }

    fn poll_response(status: &str, stdout: &str, stderr: &str) -> String {
        format!(
            "__WISP_RUN_STATUS__:{status}\n__WISP_STDOUT__\n{stdout}\n__WISP_STDERR__\n{stderr}\n"
        )
    }

    fn test_handle(run_id: &str, confirmed: bool) -> RemoteRunHandle {
        RemoteRunHandle::SshDirect {
            connection: crate::ssh_hosts::SshConnection {
                alias: "gpu".into(),
                user: None,
                port: None,
                identity_file: None,
            },
            workdir: format!(".wisp-science/runs/{run_id}"),
            token: "test-token".into(),
            pgid: confirmed.then_some(4242),
            start_time: confirmed.then_some(999),
        }
    }

    async fn wait_for_terminal(store: &wisp_store::Store, run_id: &str) -> wisp_store::RunRecord {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let run = store.get_run(run_id).await.unwrap().unwrap();
                if run.status.is_terminal() {
                    return run;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }
}
