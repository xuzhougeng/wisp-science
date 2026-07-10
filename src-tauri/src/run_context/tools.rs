use super::{RunManager, SubmitRunRequest};
use wisp_llm::ToolSchema;
use wisp_tools::{Tool, ToolEnv, ToolResult};

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
