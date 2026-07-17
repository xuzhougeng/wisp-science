//! Persistent `python` and `r` tools backed by `RuntimeManager`.

use crate::{
    KernelResp, RuntimeEvent, RuntimeKey, RuntimeManager, LOCAL_CONTEXT_ID, MAX_CODE_BYTES,
};
use async_trait::async_trait;
use serde_json::json;
use wisp_llm::ToolSchema;
use wisp_tools::{Tool, ToolEnv, ToolEvent, ToolResult};

pub struct ReplTool {
    manager: RuntimeManager,
    project_id: String,
}

pub struct RTool {
    manager: RuntimeManager,
    project_id: String,
}

const PYTHON_TOOL_DESCRIPTION: &str = "Execute Python code in a persistent REPL. Variables, imports, and loaded data persist per project and execution context. Return values of expressions are printed. Paths are interpreted inside the selected context. Use this for analysis, data loading, plotting, and computation when required packages already exist. Do not use this as a package installer; if dependencies are missing, set up a project-local pixi environment or use local-env-setup first.";
const R_TOOL_DESCRIPTION: &str = "Execute R code in a persistent REPL. Variables, libraries, and loaded data persist per project and execution context. The final visible value is printed. Paths are interpreted inside the selected context. Write plots explicitly with png(), pdf(), ggsave(), or another file device. Rscript and the jsonlite package must already exist in that context; this tool does not install packages.";

impl ReplTool {
    pub fn new(manager: RuntimeManager, project_id: impl Into<String>) -> Self {
        Self {
            manager,
            project_id: project_id.into(),
        }
    }
}

impl RTool {
    pub fn new(manager: RuntimeManager, project_id: impl Into<String>) -> Self {
        Self {
            manager,
            project_id: project_id.into(),
        }
    }
}

fn context_id(args: &serde_json::Value) -> Result<&str, &'static str> {
    match args.get("context_id") {
        None => Ok(LOCAL_CONTEXT_ID),
        Some(value) => value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or("argument 'context_id' must be a non-empty string"),
    }
}

fn code_arg(args: &serde_json::Value) -> Result<String, String> {
    let code = args
        .get("code")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "missing required argument 'code'".to_string())?;
    if code.len() > MAX_CODE_BYTES {
        return Err(format!(
            "argument 'code' exceeds {MAX_CODE_BYTES} byte limit"
        ));
    }
    Ok(code.to_string())
}

/// Render a kernel response the way the `python`/`r` tools do, so a user-driven
/// run from the UI reads identically to an agent-driven one.
pub fn format_response(resp: &KernelResp) -> String {
    let mut out = String::new();
    if !resp.stdout.is_empty() {
        out.push_str(&resp.stdout);
    }
    if !resp.stderr.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("[stderr] ");
        out.push_str(&resp.stderr);
    }
    if let Some(err) = &resp.error {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("[error] ");
        out.push_str(err);
    }
    if out.is_empty() {
        out = "(no output)".into();
    }
    out
}

async fn run_runtime(
    manager: &RuntimeManager,
    key: RuntimeKey,
    code: String,
    language: &'static str,
    env: &dyn ToolEnv,
) -> ToolResult {
    let mut execution = match manager.execute(&key, env.project_root(), code).await {
        Ok(execution) => execution,
        Err(error) => return ToolResult::fail(format!("{language} error: {error}")),
    };
    let mut cancel_poll = tokio::time::interval(std::time::Duration::from_millis(50));
    loop {
        tokio::select! {
            event = execution.recv() => match event {
                Some(RuntimeEvent::Stdout(chunk)) => {
                    env.emit(ToolEvent::Stdout { chunk }).await;
                }
                Some(RuntimeEvent::Finished(Ok(response))) => {
                    let success = response.error.is_none();
                    return ToolResult {
                        success,
                        content: format_response(&response),
                        image: None,
                    };
                }
                Some(RuntimeEvent::Finished(Err(error))) => {
                    return ToolResult::fail(format!("{language} error: {error}"));
                }
                None => {
                    return ToolResult::fail(format!(
                        "{language} error: runtime ended before returning a result"
                    ));
                }
            },
            _ = cancel_poll.tick() => {
                if env.is_cancelled() {
                    // Dropping this receiver abandons only the caller. The
                    // manager-owned protocol task still drains the cell.
                    return ToolResult::fail(format!("{language} error: interrupted by user"));
                }
            }
        }
    }
}

#[async_trait]
impl Tool for ReplTool {
    fn name(&self) -> &str {
        "python"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "python",
            PYTHON_TOOL_DESCRIPTION,
            json!({
                "type": "object",
                "properties": {
                    "code": { "type": "string", "description": "Python code to execute (statements or a single expression)" },
                    "context_id": { "type": "string", "description": "Execution context id; defaults to local (for example local, ssh:gpu, or wsl:Ubuntu)" }
                },
                "required": ["code"]
            }),
        )
    }

    fn preview(&self, args: &serde_json::Value) -> String {
        let context = context_id(args).unwrap_or("invalid");
        let code = args
            .get("code")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        format!("[python @ {context}] {code}")
    }

    async fn run(&self, args: &serde_json::Value, env: &dyn ToolEnv) -> ToolResult {
        let code = match code_arg(args) {
            Ok(code) => code,
            Err(error) => return ToolResult::fail(error),
        };
        let context_id = match context_id(args) {
            Ok(context_id) => context_id,
            Err(error) => return ToolResult::fail(error),
        };
        run_runtime(
            &self.manager,
            RuntimeKey::python(&self.project_id, context_id),
            code,
            "python",
            env,
        )
        .await
    }
}

#[async_trait]
impl Tool for RTool {
    fn name(&self) -> &str {
        "r"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "r",
            R_TOOL_DESCRIPTION,
            json!({
                "type": "object",
                "properties": {
                    "code": { "type": "string", "description": "R code to execute (one or more expressions)" },
                    "context_id": { "type": "string", "description": "Execution context id; defaults to local (for example local, ssh:gpu, or wsl:Ubuntu)" }
                },
                "required": ["code"]
            }),
        )
    }

    fn preview(&self, args: &serde_json::Value) -> String {
        let context = context_id(args).unwrap_or("invalid");
        let code = args
            .get("code")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        format!("[r @ {context}] {code}")
    }

    async fn run(&self, args: &serde_json::Value, env: &dyn ToolEnv) -> ToolResult {
        let code = match code_arg(args) {
            Ok(code) => code,
            Err(error) => return ToolResult::fail(error),
        };
        let context_id = match context_id(args) {
            Ok(context_id) => context_id,
            Err(error) => return ToolResult::fail(error),
        };
        run_runtime(
            &self.manager,
            RuntimeKey::r(&self.project_id, context_id),
            code,
            "r",
            env,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::{code_arg, context_id, PYTHON_TOOL_DESCRIPTION, R_TOOL_DESCRIPTION};
    use crate::MAX_CODE_BYTES;

    #[test]
    fn python_description_keeps_package_setup_out_of_the_repl() {
        assert!(PYTHON_TOOL_DESCRIPTION.contains("Do not use this as a package installer"));
        assert!(PYTHON_TOOL_DESCRIPTION.contains("project-local pixi"));
        assert!(PYTHON_TOOL_DESCRIPTION.contains("local-env-setup"));
    }

    #[test]
    fn r_description_requires_existing_runtime_dependencies_and_explicit_plots() {
        assert!(R_TOOL_DESCRIPTION.contains("Rscript"));
        assert!(R_TOOL_DESCRIPTION.contains("jsonlite"));
        assert!(R_TOOL_DESCRIPTION.contains("png()"));
        assert!(R_TOOL_DESCRIPTION.contains("does not install packages"));
    }

    #[test]
    fn context_defaults_to_local_and_rejects_blank_values() {
        assert_eq!(
            context_id(&serde_json::json!({"code": "1"})).unwrap(),
            "local"
        );
        assert!(context_id(&serde_json::json!({"context_id": "  "})).is_err());
        assert_eq!(
            context_id(&serde_json::json!({"context_id": " ssh:gpu "})).unwrap(),
            "ssh:gpu"
        );
    }

    #[test]
    fn code_size_is_rejected_before_runtime_dispatch() {
        let args = serde_json::json!({"code": "x".repeat(MAX_CODE_BYTES + 1)});
        assert!(code_arg(&args).unwrap_err().contains("byte limit"));
    }
}
