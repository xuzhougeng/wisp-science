use super::{
    RunManager, RunPreflightReport, RunPreflightSpec, RunPreflightStatus, SubmitRunRequest,
};
use wisp_llm::ToolSchema;
use wisp_tools::{Approval, Tool, ToolEnv, ToolResult};

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
            "Submit a persisted background Run in an execution context (`local`, `ssh:<alias>`, or `wsl:<distro>`). For Python/R work, declare a preflight to check the interpreter, explicit import modules/packages, project-relative files, and syntax before submission. Preflight never installs packages or executes the requested command as a dry run. Set wait_for_completion=true for direct model-free waiting, or submit normally and call monitor_run with the returned Run id to show an inline live card. After submission, call monitor_run directly without announcing that the Run was submitted or that you are waiting; the Run card communicates that state. If monitor_run returns wait_interrupted, the Run is still running: respond, then call monitor_run again with the same id. Do not resubmit. Never poll with get_run.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "context_id": { "type": "string", "description": "Execution context id, e.g. local, ssh:gpu, wsl:Ubuntu" },
                    "command": { "type": "string", "description": "Command to execute in that context" },
                    "title": { "type": "string", "description": "Short run title" },
                    "timeout_secs": { "type": "integer", "description": "Job wall timeout in seconds: 1s..7d (default 4h) for local, WSL, and SSH" },
                    "wait_for_completion": { "type": "boolean", "description": "Suspend this tool until the Run reaches a terminal state or mid-turn user guidance arrives, without consuming model tokens or repeatedly calling get_run (default false). If wait_interrupted, respond then call monitor_run to resume waiting." },
                    "preflight": {
                        "type": "object",
                        "description": "Safe declarative Python/R environment checks performed before Run submission",
                        "properties": {
                            "language": { "type": "string", "enum": ["python", "r"] },
                            "packages": {
                                "type": "array",
                                "description": "Explicit Python import module names or R package names; no packages are installed",
                                "items": { "type": "string" },
                                "maxItems": 32
                            },
                            "paths": {
                                "type": "array",
                                "description": "Project-relative files that must exist",
                                "items": { "type": "string" },
                                "maxItems": 32
                            },
                            "syntax_paths": {
                                "type": "array",
                                "description": "Project-relative .py/.R files to parse without executing",
                                "items": { "type": "string" },
                                "maxItems": 32
                            },
                            "allow_warnings": {
                                "type": "boolean",
                                "description": "Proceed after non-fatal preflight warnings only after the user has approved them (default false)"
                            }
                        },
                        "required": ["language"]
                    },
                    "input_paths": {
                        "type": "array",
                        "description": "Optional project-relative files bound as exact Run inputs; SSH also stages them flat into the remote workdir",
                        "items": { "type": "string" }
                    },
                    "output_specs": {
                        "type": "array",
                        "description": "Optional output specs selecting which results are registered (and, for SSH Runs, downloaded back into the project after success with checksum verification). Point globs at final products only; intermediate files stay unregistered and are reclaimed by cleanup. SSH globs are workdir-relative; explicit ssh:// URIs register remote references without download",
                        "items": {
                            "type": "object",
                            "properties": {
                                "glob": { "type": "string" },
                                "kind": { "type": "string" },
                                "residency": { "type": "string", "enum": ["local", "remote", "auto"] },
                                "logical_key": { "type": "string", "description": "Stable logical output identity; defaults to the matched project-relative path" },
                                "max_file_mb": { "type": "integer" },
                                "max_total_mb": { "type": "integer" },
                                "bundle": { "type": "boolean", "description": "Pack all matched files into one tar.gz registered as a single artifact. Required when a glob matches many files (e.g. assembly intermediates)" }
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
        if args
            .get("wait_for_completion")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            format!("{context}: {command} · wait")
        } else {
            format!("{context}: {command}")
        }
    }

    async fn run(&self, args: &serde_json::Value, env: &dyn ToolEnv) -> ToolResult {
        let request: SubmitRunRequest = match serde_json::from_value(args.clone()) {
            Ok(req) => req,
            Err(e) => return ToolResult::fail(format!("run_in_context args error: {e}")),
        };
        if crate::exploration_isolation::is_host_local_context(&request.context_id) {
            let scope = match self.frame_id.as_deref() {
                Some(frame_id) => match self.store.frame_state_scope(frame_id).await {
                    Ok(Some(scope)) => scope,
                    Ok(None) => {
                        return ToolResult::fail(
                            "run_in_context could not resolve the conversation scope",
                        )
                        .stop_batch()
                    }
                    Err(error) => {
                        return ToolResult::fail(format!(
                            "run_in_context could not resolve the conversation scope: {error}"
                        ))
                        .stop_batch()
                    }
                },
                None => wisp_store::StateScope::mainline(self.project_id.clone()),
            };
            match crate::exploration_isolation::boundary_for_scope(&self.store, &scope).await {
                Ok(Some(boundary)) => {
                    if let Err(error) = boundary.check_local_source(&request.command) {
                        return ToolResult::fail(error).stop_batch();
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    return ToolResult::fail(format!(
                        "run_in_context could not establish the exploration boundary: {error}"
                    ))
                    .stop_batch()
                }
            }
        }
        if let Err(error) = crate::ssh_guard::preflight_shell(&request.command) {
            return ToolResult::fail(format!(
                "{error} For server-to-server copies, use `transfer_between_contexts`; use \
                 `configure_ssh_trust` first when the user approves a direct trust edge."
            ));
        }
        if !env.danger_auto_approve() {
            let danger = wisp_tools::safety::check_command_safety(&request.command);
            let exploration_external = if request.context_id != "local" {
                match self.frame_id.as_deref() {
                    Some(frame_id) => match self.store.frame_state_scope(frame_id).await {
                        Ok(Some(wisp_store::StateScope::Exploration { .. })) => true,
                        Ok(_) => false,
                        Err(error) => {
                            return ToolResult::fail(format!(
                                "run_in_context could not resolve the conversation scope: {error}"
                            ))
                            .stop_batch();
                        }
                    },
                    None => false,
                }
            } else {
                false
            };
            if danger.is_some() || exploration_external {
                let mut warnings = Vec::new();
                if exploration_external {
                    warnings.push(format!(
                        "This exploration will execute on external context '{}'; remote files, jobs, and services cannot be rolled back when the exploration is discarded.",
                        request.context_id
                    ));
                }
                if let Some(danger) = danger {
                    warnings.push(format!(
                        "Dangerous command detected ({}): {}",
                        danger.label(),
                        request.command
                    ));
                }
                if !env.confirm(&warnings.join("\n")).await {
                    return ToolResult::fail("error: User denied action").stop_batch();
                }
            }
        }
        let mut preflight = match args.get("preflight") {
            Some(value) => match serde_json::from_value::<RunPreflightSpec>(value.clone()) {
                Ok(spec) => Some(spec),
                Err(error) => {
                    return ToolResult::fail(format!(
                        "run_in_context preflight args error: {error}"
                    ))
                }
            },
            None => None,
        };
        if let (Some(spec), Some(input_paths)) = (&mut preflight, request.input_paths.as_ref()) {
            for path in input_paths {
                if !spec.paths.contains(path) && !spec.syntax_paths.contains(path) {
                    spec.paths.push(path.clone());
                }
            }
        }
        let preflight_report = if let Some(spec) = preflight.as_ref() {
            match self
                .manager
                .preflight(&self.store, &request.context_id, env.project_root(), spec)
                .await
            {
                Ok(report) if report.status == RunPreflightStatus::Failed => {
                    return ToolResult::fail(preflight_blocked_result(&report, false))
                }
                Ok(report)
                    if report.status == RunPreflightStatus::Warning && !spec.allow_warnings =>
                {
                    return ToolResult::fail(preflight_blocked_result(&report, true));
                }
                Ok(report) => Some(report),
                Err(error) => {
                    return ToolResult::fail(format!("run_in_context preflight error: {error}"))
                }
            }
        } else {
            None
        };
        let wait_for_completion = args
            .get("wait_for_completion")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let submission = match preflight_report.clone() {
            Some(report) => {
                self.manager
                    .submit_preflighted(
                        self.store.clone(),
                        self.project_id.clone(),
                        self.frame_id.clone(),
                        request,
                        Some(env.project_root().to_path_buf()),
                        report,
                    )
                    .await
            }
            None => {
                self.manager
                    .submit(
                        self.store.clone(),
                        self.project_id.clone(),
                        self.frame_id.clone(),
                        request,
                        Some(env.project_root().to_path_buf()),
                    )
                    .await
            }
        };
        match submission {
            Ok(res) if wait_for_completion => {
                match wait_for_terminal(&self.store, &res.run_id, env).await {
                    Ok((run, outcome)) => run_wait_result(run, outcome),
                    Err(error) => ToolResult::fail(format!("run_in_context wait error: {error}")),
                }
            }
            Ok(res) => {
                let mut value = serde_json::to_value(res).unwrap_or_default();
                if let Some(report) = preflight_report {
                    value["preflight"] = serde_json::to_value(report).unwrap_or_default();
                }
                ToolResult::ok(value.to_string())
            }
            Err(e) => ToolResult::fail(format!("run_in_context error: {e}")),
        }
    }
}

fn preflight_blocked_result(report: &RunPreflightReport, requires_confirmation: bool) -> String {
    serde_json::json!({
        "run_submitted": false,
        "preflight": report,
        "requires_confirmation": requires_confirmation,
        "next_action": if requires_confirmation {
            "Show the warning to the user. Only after explicit approval, repeat with preflight.allow_warnings=true."
        } else {
            "Fix the failed preflight checks before submitting the Run."
        }
    })
    .to_string()
}

pub struct GetRunTool {
    store: wisp_store::Store,
    scope: wisp_store::StateScope,
}

impl GetRunTool {
    pub fn new(store: wisp_store::Store, project_id: String) -> Self {
        Self {
            store,
            scope: wisp_store::StateScope::mainline(project_id),
        }
    }

    pub fn new_in_scope(store: wisp_store::Store, scope: wisp_store::StateScope) -> Self {
        Self { store, scope }
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
            "Read one immediate status snapshot for a Run. Never call this repeatedly to wait; call monitor_run for live monitoring until completion or mid-turn guidance.",
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
        match self.store.run_visible_in_scope(run_id, &self.scope).await {
            Ok(true) => {}
            Ok(false) => return ToolResult::fail("Run does not belong to this state scope"),
            Err(error) => return ToolResult::fail(format!("get_run error: {error}")),
        }
        match self.store.get_run(run_id).await {
            Ok(Some(run)) => {
                let active = !run.status.is_terminal();
                let mut value = serde_json::to_value(run).unwrap_or_default();
                if active {
                    value["next_action"] = serde_json::Value::String(
                        "Do not poll with get_run. Call monitor_run with this run_id to wait; if wait_interrupted, respond then call monitor_run again."
                            .into(),
                    );
                }
                ToolResult::ok(value.to_string())
            }
            Ok(None) => ToolResult::fail("Run not found"),
            Err(error) => ToolResult::fail(format!("get_run error: {error}")),
        }
    }
}

pub struct MonitorRunTool {
    store: wisp_store::Store,
    scope: wisp_store::StateScope,
}

impl MonitorRunTool {
    pub fn new(store: wisp_store::Store, project_id: String) -> Self {
        Self {
            store,
            scope: wisp_store::StateScope::mainline(project_id),
        }
    }

    pub fn new_in_scope(store: wisp_store::Store, scope: wisp_store::StateScope) -> Self {
        Self { store, scope }
    }
}

#[async_trait::async_trait]
impl Tool for MonitorRunTool {
    fn name(&self) -> &str {
        "monitor_run"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "monitor_run",
            "Monitor one existing long-running Run until it finishes or mid-turn user guidance arrives. Call this instead of repeatedly calling get_run. Wisp shows a live Run card and suspends without model calls or token use, so call monitor_run without a user-facing preamble and do not say that you are waiting or monitoring. If the result has wait_interrupted=true, the Run is still running: answer the user from this snapshot, then call monitor_run again with the same run_id. Do not resubmit the Run. Use cancel_run only when the user asked to stop.",
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

    async fn run(&self, args: &serde_json::Value, env: &dyn ToolEnv) -> ToolResult {
        let Some(run_id) = args.get("run_id").and_then(|value| value.as_str()) else {
            return ToolResult::fail("monitor_run requires run_id");
        };
        match self.store.run_visible_in_scope(run_id, &self.scope).await {
            Ok(true) => {}
            Ok(false) => return ToolResult::fail("Run does not belong to this state scope"),
            Err(error) => return ToolResult::fail(format!("monitor_run error: {error}")),
        }
        match wait_for_terminal(&self.store, run_id, env).await {
            Ok((run, outcome)) => run_wait_result(run, outcome),
            Err(error) => ToolResult::fail(format!("monitor_run error: {error}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaitOutcome {
    Terminal,
    Detached,
    GuidanceInterrupt,
}

const WAIT_INTERRUPTED_NEXT_ACTION: &str = "The Run is still executing and was not cancelled or \
     resubmitted. Respond to the user's mid-turn message using this snapshot (status, heartbeat, \
     log tails). Then call monitor_run again with the same run_id to resume waiting. Call \
     cancel_run only if the user asked to stop the Run.";

async fn wait_for_terminal(
    store: &wisp_store::Store,
    run_id: &str,
    env: &dyn ToolEnv,
) -> Result<(wisp_store::RunRecord, WaitOutcome), String> {
    loop {
        let run = store
            .get_run(run_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Run not found: {run_id}"))?;
        if run.status.is_terminal() {
            return Ok((run, WaitOutcome::Terminal));
        }
        if env.is_cancelled() {
            return Ok((run, WaitOutcome::Detached));
        }
        if env.guidance_pending() {
            return Ok((run, WaitOutcome::GuidanceInterrupt));
        }
        tokio::time::sleep(if cfg!(test) {
            std::time::Duration::from_millis(10)
        } else {
            std::time::Duration::from_secs(1)
        })
        .await;
    }
}

fn run_wait_result(run: wisp_store::RunRecord, outcome: WaitOutcome) -> ToolResult {
    let succeeded = run.status == wisp_store::RunStatus::Succeeded;
    let cleanable = outcome == WaitOutcome::Terminal
        && run.status.is_terminal()
        && run.remote_workdir.is_some()
        && run.cleaned_at.is_none()
        && run.kind != "file_transfer";
    let mut value = serde_json::to_value(run).unwrap_or_default();
    match outcome {
        WaitOutcome::Detached => {
            value["wait_detached"] = serde_json::Value::Bool(true);
        }
        WaitOutcome::GuidanceInterrupt => {
            value["wait_interrupted"] = serde_json::Value::Bool(true);
            value["next_action"] = serde_json::Value::String(WAIT_INTERRUPTED_NEXT_ACTION.into());
        }
        WaitOutcome::Terminal if cleanable => {
            value["next_action"] = serde_json::Value::String(
                "When the server workspace is no longer needed, call cleanup_run_workspace to \
                 reclaim it (harvested outputs and logs stay in the project)."
                    .into(),
            );
        }
        WaitOutcome::Terminal => {}
    }
    let content = value.to_string();
    match outcome {
        WaitOutcome::Detached | WaitOutcome::GuidanceInterrupt => ToolResult::ok(content),
        WaitOutcome::Terminal if succeeded => ToolResult::ok(content),
        WaitOutcome::Terminal => ToolResult::fail(content),
    }
}

pub struct HarvestRunTool {
    store: wisp_store::Store,
    manager: RunManager,
    scope: wisp_store::StateScope,
}

impl HarvestRunTool {
    pub fn new_in_scope(
        store: wisp_store::Store,
        manager: RunManager,
        scope: wisp_store::StateScope,
    ) -> Self {
        Self {
            store,
            manager,
            scope,
        }
    }
}

#[async_trait::async_trait]
impl Tool for HarvestRunTool {
    fn name(&self) -> &str {
        "harvest_run"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "harvest_run",
            "Retry output harvest for a succeeded Run whose declared output_specs were never registered (for example the automatic post-run download failed or the app closed mid-pull). Downloads spec-matched remote outputs with checksum verification and registers them as project artifacts.",
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
            return ToolResult::fail("harvest_run requires run_id");
        };
        match self.store.run_visible_in_scope(run_id, &self.scope).await {
            Ok(true) => {}
            Ok(false) => return ToolResult::fail("Run does not belong to this state scope"),
            Err(error) => return ToolResult::fail(format!("harvest_run error: {error}")),
        }
        match self.manager.harvest_run(&self.store, run_id).await {
            Ok(harvested) => ToolResult::ok(
                serde_json::json!({ "run_id": run_id, "harvested": harvested }).to_string(),
            ),
            Err(error) => ToolResult::fail(format!("harvest_run error: {error}")),
        }
    }
}

pub struct CleanupRunWorkspaceTool {
    store: wisp_store::Store,
    manager: RunManager,
    scope: wisp_store::StateScope,
}

impl CleanupRunWorkspaceTool {
    pub fn new_in_scope(
        store: wisp_store::Store,
        manager: RunManager,
        scope: wisp_store::StateScope,
    ) -> Self {
        Self {
            store,
            manager,
            scope,
        }
    }
}

#[async_trait::async_trait]
impl Tool for CleanupRunWorkspaceTool {
    fn name(&self) -> &str {
        "cleanup_run_workspace"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "cleanup_run_workspace",
            "Delete a finished Run's server-side workspace (inputs, logs, intermediate files). Requires a terminal Run; a succeeded Run with declared output_specs must be harvested first so results are never lost. Registered artifacts and run logs in the project are unaffected. Idempotent.",
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
            return ToolResult::fail("cleanup_run_workspace requires run_id");
        };
        match self.store.run_visible_in_scope(run_id, &self.scope).await {
            Ok(true) => {}
            Ok(false) => return ToolResult::fail("Run does not belong to this state scope"),
            Err(error) => return ToolResult::fail(format!("cleanup_run_workspace error: {error}")),
        }
        match self
            .manager
            .cleanup_run_workspace(&self.store, run_id, false)
            .await
        {
            Ok(run) => ToolResult::ok(serde_json::to_string(&run).unwrap_or_default()),
            Err(error) => ToolResult::fail(format!("cleanup_run_workspace error: {error}")),
        }
    }
}

pub struct ListRemoteFilesTool {
    store: wisp_store::Store,
    project_id: String,
    frame_id: Option<String>,
}

impl ListRemoteFilesTool {
    pub fn new(store: wisp_store::Store, project_id: String, frame_id: Option<String>) -> Self {
        Self {
            store,
            project_id,
            frame_id,
        }
    }
}

async fn selected_ssh_context_for_tools(
    store: &wisp_store::Store,
    frame_id: Option<&str>,
    context_id: &str,
) -> Result<wisp_store::ExecutionContext, String> {
    let context = store
        .get_execution_context(context_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Execution context not found: {context_id}"))?;
    if context.kind != wisp_store::ExecutionContextKind::Ssh {
        return Err(format!("Execution context is not SSH: {context_id}"));
    }
    if let Some(frame_id) = frame_id {
        if !store
            .session_execution_context_enabled(frame_id, context_id)
            .await
            .map_err(|e| e.to_string())?
        {
            return Err(format!(
                "Execution context {context_id} is not selected for this session"
            ));
        }
    }
    Ok(context)
}

#[async_trait::async_trait]
impl Tool for ListRemoteFilesTool {
    fn name(&self) -> &str {
        "list_remote_files"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "list_remote_files",
            "List the files this project placed on one SSH server (run input staging, uploads, and harvest-persisted outputs left on the server). Current successful uploads stay active (they are the user's dataset). Replaced means a newer upload owns that path (ledger-only). Orphan means a failed/cancelled partial or an unreferenced persist file — safe to delete. Use remove_remote_files to delete retracted files.",
            serde_json::json!({
                "type": "object",
                "properties": { "context_id": { "type": "string", "description": "SSH context id, e.g. ssh:gpu" } },
                "required": ["context_id"]
            }),
        )
    }

    fn preview(&self, args: &serde_json::Value) -> String {
        args.get("context_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .into()
    }

    async fn run(&self, args: &serde_json::Value, _env: &dyn ToolEnv) -> ToolResult {
        let Some(context_id) = args.get("context_id").and_then(|value| value.as_str()) else {
            return ToolResult::fail("list_remote_files requires context_id");
        };
        if let Err(error) =
            selected_ssh_context_for_tools(&self.store, self.frame_id.as_deref(), context_id).await
        {
            return ToolResult::fail(format!("list_remote_files error: {error}"));
        }
        match super::remote_files::list_remote_files(&self.store, &self.project_id, context_id)
            .await
        {
            Ok(files) => ToolResult::ok(
                serde_json::json!({ "context_id": context_id, "files": files }).to_string(),
            ),
            Err(error) => ToolResult::fail(format!("list_remote_files error: {error}")),
        }
    }
}

pub struct RemoveRemoteFilesTool {
    store: wisp_store::Store,
    manager: RunManager,
    project_id: String,
    frame_id: Option<String>,
}

impl RemoveRemoteFilesTool {
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
impl Tool for RemoveRemoteFilesTool {
    fn name(&self) -> &str {
        "remove_remote_files"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "remove_remote_files",
            "Delete ledgered files from one SSH server by their list_remote_files entry ids. Orphan entries delete the remote bytes. Replaced entries are closed in the ledger only — they share a path with the current file, so deleting them must not rm that path. Active entries are refused unless the UI passes force after explicit user confirmation.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "context_id": { "type": "string" },
                    "ids": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["context_id", "ids"]
            }),
        )
    }

    fn minimum_approval(&self) -> Approval {
        Approval::Ask
    }

    fn preview(&self, args: &serde_json::Value) -> String {
        let context = args
            .get("context_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let count = args
            .get("ids")
            .and_then(|value| value.as_array())
            .map(|ids| ids.len())
            .unwrap_or(0);
        format!("{context}: {count} file(s)")
    }

    async fn run(&self, args: &serde_json::Value, _env: &dyn ToolEnv) -> ToolResult {
        let Some(context_id) = args.get("context_id").and_then(|value| value.as_str()) else {
            return ToolResult::fail("remove_remote_files requires context_id");
        };
        let ids: Vec<String> = args
            .get("ids")
            .and_then(|value| value.as_array())
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| id.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let context =
            match selected_ssh_context_for_tools(&self.store, self.frame_id.as_deref(), context_id)
                .await
            {
                Ok(context) => context,
                Err(error) => {
                    return ToolResult::fail(format!("remove_remote_files error: {error}"))
                }
            };
        match super::remote_files::remove_remote_files(
            &self.store,
            self.manager.runner.as_ref(),
            &self.project_id,
            &context,
            &ids,
            false,
        )
        .await
        {
            Ok(removed) => ToolResult::ok(
                serde_json::json!({ "context_id": context_id, "removed": removed }).to_string(),
            ),
            Err(error) => ToolResult::fail(format!("remove_remote_files error: {error}")),
        }
    }
}

pub struct CancelRunTool {
    store: wisp_store::Store,
    manager: RunManager,
    scope: wisp_store::StateScope,
}

impl CancelRunTool {
    pub fn new(store: wisp_store::Store, manager: RunManager, project_id: String) -> Self {
        Self {
            store,
            manager,
            scope: wisp_store::StateScope::mainline(project_id),
        }
    }

    pub fn new_in_scope(
        store: wisp_store::Store,
        manager: RunManager,
        scope: wisp_store::StateScope,
    ) -> Self {
        Self {
            store,
            manager,
            scope,
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
        match self.store.run_state_scope(run_id).await {
            Ok(Some(scope)) if scope == self.scope => {}
            Ok(Some(_)) => return ToolResult::fail("Run does not belong to this state scope"),
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
