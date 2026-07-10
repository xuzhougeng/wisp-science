use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCommand {
    pub context_id: String,
    pub program: String,
    pub args: Vec<String>,
    pub script: String,
    pub cwd: Option<PathBuf>,
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
        if let Some(cwd) = &command.cwd {
            cmd.current_dir(cwd);
        }
        wisp_tools::process::hide_console_async(&mut cmd);
        let child = cmd
            .spawn()
            .map_err(|e| format!("failed to spawn {}: {e}", command.program))?;
        let output = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| format!("run_in_context timed out after {}s", timeout.as_secs()))?
            .map_err(|e| format!("run_in_context wait failed: {e}"))?;
        Ok(RunCommandOutput {
            exit_code: output.status.code().unwrap_or(-1) as i64,
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

#[derive(Clone)]
pub struct RunManager {
    runner: Arc<dyn RunCommandRunner>,
    active: Arc<Mutex<HashMap<String, tokio::task::AbortHandle>>>,
}

impl RunManager {
    pub fn new() -> Self {
        Self::with_runner(Arc::new(ProcessRunRunner))
    }

    pub fn with_runner(runner: Arc<dyn RunCommandRunner>) -> Self {
        Self {
            runner,
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn recover(&self, store: &wisp_store::Store) -> Result<u64, String> {
        store
            .mark_active_runs_lost()
            .await
            .map_err(|e| e.to_string())
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
        )
        .await?;
        let run_id = prepared.run_id.clone();
        let task_store = store.clone();
        let runner = self.runner.clone();
        let active = self.active.clone();
        let cleanup_id = run_id.clone();
        let task_run_id = cleanup_id.clone();
        let task = tokio::spawn(async move {
            let result = async {
                task_store
                    .update_run_status(&prepared.run_id, wisp_store::RunStatus::Running)
                    .await
                    .map_err(|e| e.to_string())?;
                let output = runner.run(prepared.command.clone(), prepared.timeout).await;
                record_run_outcome(&task_store, &prepared, output).await
            }
            .await;
            if let Err(error) = result {
                tracing::warn!(run_id = %task_run_id, "background run failed: {error}");
            }
        });
        let handle = task.abort_handle();
        self.active.lock().await.insert(run_id.clone(), handle);
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
        })
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
        if let Some(handle) = self.active.lock().await.remove(run_id) {
            handle.abort();
        }
        store
            .finish_run(run_id, wisp_store::RunStatus::Cancelled, None)
            .await
            .map_err(|e| e.to_string())
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
}

async fn create_run_record(
    store: &wisp_store::Store,
    project_id: &str,
    frame_id: Option<&str>,
    request: SubmitRunRequest,
    cwd: Option<PathBuf>,
    initial_status: wisp_store::RunStatus,
) -> Result<PreparedRun, String> {
    let command = request.command.trim();
    if command.is_empty() {
        return Err("command is required".into());
    }
    let ctx = store
        .get_execution_context(&request.context_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Execution context not found: {}", request.context_id))?;
    let timeout = Duration::from_secs(request.timeout_secs.unwrap_or(60).clamp(1, 300));
    let run_id = uuid::Uuid::new_v4().to_string();
    let mut run = wisp_store::RunRecord::new(
        &run_id,
        project_id,
        &ctx.id,
        request
            .title
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(command),
        "command",
    );
    run.frame_id = frame_id.map(Into::into);
    run.command = Some(command.into());
    let output_specs = request.output_specs.unwrap_or_default();
    run.output_specs_json = serde_json::to_string(&output_specs).map_err(|e| e.to_string())?;
    store.create_run(&run).await.map_err(|e| e.to_string())?;
    store
        .update_run_status(&run_id, initial_status)
        .await
        .map_err(|e| e.to_string())?;
    Ok(PreparedRun {
        run_id,
        project_id: project_id.into(),
        command: build_run_command(&ctx, command, cwd.clone()),
        timeout,
        output_specs,
        frame_id: frame_id.map(Into::into),
        harvest_root: cwd,
    })
}

async fn record_run_outcome(
    store: &wisp_store::Store,
    prepared: &PreparedRun,
    output: Result<RunCommandOutput, String>,
) -> Result<SubmitRunResponse, String> {
    match output {
        Ok(out) => {
            let stdout_tail = tail(&out.stdout);
            let stderr_tail = tail(&out.stderr);
            store
                .update_run_output(&prepared.run_id, Some(&stdout_tail), Some(&stderr_tail))
                .await
                .map_err(|e| e.to_string())?;
            let status = if out.exit_code == 0 {
                wisp_store::RunStatus::Succeeded
            } else {
                wisp_store::RunStatus::Failed
            };
            store
                .finish_run(&prepared.run_id, status, Some(out.exit_code))
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
            })
        }
        Err(e) => {
            let stderr_tail = tail(&e);
            store
                .update_run_output(&prepared.run_id, None, Some(&stderr_tail))
                .await
                .map_err(|err| err.to_string())?;
            store
                .finish_run(&prepared.run_id, wisp_store::RunStatus::Failed, Some(-1))
                .await
                .map_err(|err| err.to_string())?;
            Ok(SubmitRunResponse {
                run_id: prepared.run_id.clone(),
                status: wisp_store::RunStatus::Failed,
                exit_code: Some(-1),
                stdout_tail: None,
                stderr_tail: Some(stderr_tail),
            })
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
    )
    .await?;
    let output = runner.run(prepared.command.clone(), prepared.timeout).await;
    record_run_outcome(store, &prepared, output).await
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
            "Submit a persisted background Run in an execution context (`local`, `ssh:<alias>`, or `wsl:<distro>`). Use this instead of shell for research runs that should be tracked.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "context_id": { "type": "string", "description": "Execution context id, e.g. local, ssh:gpu, wsl:Ubuntu" },
                    "command": { "type": "string", "description": "Command to execute in that context" },
                    "title": { "type": "string", "description": "Short run title" },
                    "timeout_secs": { "type": "integer", "description": "Bounded timeout in seconds, clamped to 1..300" },
                    "output_specs": {
                        "type": "array",
                        "description": "Optional harvest specs for files produced by the run",
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
            "Read the current status and output tails of a persisted Run.",
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
            "Cancel a submitted or running Run in this project.",
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
            Ok(()) => ToolResult::ok(format!("cancelled {run_id}")),
            Err(error) => ToolResult::fail(format!("cancel_run error: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
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

    #[test]
    fn tail_preserves_utf8_boundaries() {
        let s = format!("{}{}", "a".repeat(3999), "科研");
        let out = tail(&s);
        assert!(out.starts_with('a') || out.starts_with('科'));
        assert!(out.ends_with("科研"));
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
}
