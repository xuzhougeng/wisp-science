use super::{
    RunManager, RunPreflightReport, RunPreflightSpec, RunPreflightStatus, SubmitRunRequest,
};
use wisp_llm::ToolSchema;
use wisp_tools::{Tool, ToolEnv, ToolResult};

/// Structured view of a Run attached to tool results and live progress for
/// the host/UI. This is an explicit allowlist: the host persists the final
/// details as a `session_ui_events` row, and project export copies those
/// rows verbatim, so anything machine- or credential-adjacent — remote/local
/// workdirs, remote handles, environment snapshots, command lines,
/// interpreter paths, process-control tokens — must never appear here. Keep
/// aligned with the export scrub in `crates/wisp-store/src/project_transfer.rs`
/// (`RUN_TOOL_DETAILS_EXPORT_ALLOWLIST`), which reduces legacy full-record
/// payloads to this same field set. Rich fields stay available live through
/// the `get_run_detail` Tauri command.
#[derive(Debug, Clone, serde::Serialize)]
struct RunPresentationDetails {
    /// Opaque run identifier; already known to the model and the UI.
    id: String,
    /// Lifecycle state string ("submitted", "succeeded", …).
    status: String,
    /// User/model-provided title; already model-facing.
    title: String,
    /// Process result; already reported to the model.
    exit_code: Option<i64>,
    /// Wall-clock timestamps carry no host or path information.
    created_at: i64,
    started_at: Option<i64>,
    ended_at: Option<i64>,
    /// Output tails are already part of the model-facing summary text.
    stdout_tail: Option<String>,
    stderr_tail: Option<String>,
}

fn run_presentation_details(run: &wisp_store::RunRecord) -> serde_json::Value {
    serde_json::to_value(RunPresentationDetails {
        id: run.id.clone(),
        status: run.status.as_str().into(),
        title: run.title.clone(),
        exit_code: run.exit_code,
        created_at: run.created_at,
        started_at: run.started_at,
        ended_at: run.ended_at,
        stdout_tail: run.stdout_tail.clone(),
        stderr_tail: run.stderr_tail.clone(),
    })
    .unwrap_or_default()
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
            "Submit a persisted background Run in an execution context (`local`, `ssh:<alias>`, or `wsl:<distro>`). For Python/R work, declare a preflight to check the interpreter, explicit import modules/packages, project-relative files, and syntax before submission. Preflight never installs packages or executes the requested command as a dry run. Set wait_for_completion=true for direct model-free waiting, or submit normally and call monitor_run exactly once with the returned Run id to show an inline live card. Never poll with get_run.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "context_id": { "type": "string", "description": "Execution context id, e.g. local, ssh:gpu, wsl:Ubuntu" },
                    "command": { "type": "string", "description": "Command to execute in that context" },
                    "title": { "type": "string", "description": "Short run title" },
                    "timeout_secs": { "type": "integer", "description": "Job wall timeout in seconds: 1s..7d (default 4h) for local, WSL, and SSH" },
                    "wait_for_completion": { "type": "boolean", "description": "Suspend this tool until the Run reaches a terminal state, without consuming model tokens or repeatedly calling get_run (default false)" },
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
                        "description": "Optional output specs. SSH direct currently accepts explicit ssh:// references only",
                        "items": {
                            "type": "object",
                            "properties": {
                                "glob": { "type": "string" },
                                "kind": { "type": "string" },
                                "residency": { "type": "string", "enum": ["local", "remote", "auto"] },
                                "logical_key": { "type": "string", "description": "Stable logical output identity; defaults to the matched project-relative path" },
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
                    Ok((run, detached)) => run_wait_result(run, detached),
                    Err(error) => ToolResult::fail(format!("run_in_context wait error: {error}")),
                }
            }
            Ok(res) => {
                let preflight_status =
                    preflight_report.as_ref().map(|report| match report.status {
                        RunPreflightStatus::Passed => "passed",
                        RunPreflightStatus::Warning => "warning",
                        RunPreflightStatus::Failed => "failed",
                    });
                let mut value = serde_json::to_value(&res).unwrap_or_default();
                // The remote workdir is machine-private (see
                // RunPresentationDetails); the UI reads it live via
                // `get_run_detail` instead of the persisted details.
                if let Some(object) = value.as_object_mut() {
                    object.remove("remote_workdir");
                }
                if let Some(report) = preflight_report {
                    value["preflight"] = serde_json::to_value(report).unwrap_or_default();
                }
                // The model gets a short actionable summary; the structured
                // submission record rides `details` for the host/UI.
                let mut content = format!(
                    "Run submitted: {} (status: {}). Call monitor_run exactly once with this run_id to wait for it; never poll with get_run.",
                    res.run_id,
                    res.status.as_str()
                );
                if let Some(status) = preflight_status {
                    content.push_str(&format!(" Preflight: {status}."));
                }
                ToolResult::ok(content).with_details(value)
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
            "Read one immediate status snapshot for a Run. Never call this repeatedly to wait; call monitor_run exactly once for live monitoring until completion.",
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
                        "Do not call get_run again. Call monitor_run exactly once with this run_id."
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
            "Monitor one existing long-running Run until it finishes. Call this exactly once instead of repeatedly calling get_run. Wisp shows a live Run card, suspends the agent without model calls or token use, and resumes it with the terminal result.",
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
            Ok((run, detached)) => run_wait_result(run, detached),
            Err(error) => ToolResult::fail(format!("monitor_run error: {error}")),
        }
    }
}

async fn wait_for_terminal(
    store: &wisp_store::Store,
    run_id: &str,
    env: &dyn ToolEnv,
) -> Result<(wisp_store::RunRecord, bool), String> {
    let mut last_progress = String::new();
    loop {
        let run = store
            .get_run(run_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Run not found: {run_id}"))?;
        if run.status.is_terminal() {
            return Ok((run, false));
        }
        if env.is_cancelled() {
            return Ok((run, true));
        }
        // Live structured snapshot for the host/UI, emitted only when the
        // record actually changed so a quiet run doesn't spam progress.
        // Never enters the model context. The sanitized presentation keeps
        // machine-private Run fields out of every host channel.
        let snapshot = run_presentation_details(&run);
        let fingerprint = snapshot.to_string();
        if fingerprint != last_progress {
            last_progress = fingerprint;
            env.emit(wisp_tools::ToolEvent::Progress { details: snapshot })
                .await;
        }
        tokio::time::sleep(if cfg!(test) {
            std::time::Duration::from_millis(10)
        } else {
            std::time::Duration::from_secs(1)
        })
        .await;
    }
}

fn run_wait_result(run: wisp_store::RunRecord, detached: bool) -> ToolResult {
    let succeeded = run.status == wisp_store::RunStatus::Succeeded;
    let mut details = run_presentation_details(&run);
    if detached {
        details["wait_detached"] = serde_json::Value::Bool(true);
    }
    // The model gets a short human-readable summary; the sanitized
    // presentation rides `details` for the host/UI and never enters the
    // model context.
    let content = run_wait_summary(&run, detached);
    let result = if detached || succeeded {
        ToolResult::ok(content)
    } else {
        ToolResult::fail(content)
    };
    result.with_details(details)
}

/// Model-facing summary of a finished (or detached) wait: outcome, exit code,
/// and output tails — everything the model needs to react, nothing more.
fn run_wait_summary(run: &wisp_store::RunRecord, detached: bool) -> String {
    let mut text = if detached {
        format!(
            "Stopped monitoring Run {} (\"{}\"); it is still {} and keeps running in the background.",
            run.id,
            run.title,
            run.status.as_str()
        )
    } else {
        let mut line = format!(
            "Run {} (\"{}\") finished with status {}",
            run.id,
            run.title,
            run.status.as_str()
        );
        if let Some(exit_code) = run.exit_code {
            line.push_str(&format!(" (exit code {exit_code})"));
        }
        line.push('.');
        line
    };
    if let Some(stdout) = run
        .stdout_tail
        .as_deref()
        .filter(|tail| !tail.trim().is_empty())
    {
        text.push_str(&format!("\nstdout tail:\n{stdout}"));
    }
    if let Some(stderr) = run
        .stderr_tail
        .as_deref()
        .filter(|tail| !tail.trim().is_empty())
    {
        text.push_str(&format!("\nstderr tail:\n{stderr}"));
    }
    text
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
