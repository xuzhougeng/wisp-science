//! Built-in agent tools for Wisp, Windows-first.
//!
//! Tools implement [`tool::Tool`] and run against a [`env::ToolEnv`] the host
//! supplies. [`Registry`] bundles the built-ins, exposes their JSON schemas to
//! the LLM, and dispatches tool calls. Extra tools (Python `repl`, MCP) are
//! added with [`Registry::add`].

pub mod attempt_completion;
pub mod edit;
pub mod env;
pub mod grep;
pub mod image;
pub mod plan;
pub mod process;
pub mod read;
pub mod safety;
pub mod search;
pub mod shell;
pub mod tool;
pub mod write;

pub use env::{
    Approval, ConfirmDecision, DomainConfirmationRequest, ImageData, ToolEnv, ToolEvent, ToolResult,
};
pub use tool::Tool;

use serde_json::Value;
use wisp_llm::ToolSchema;

/// The built-in tool set plus any extras (repl, MCP) registered later.
pub struct Registry {
    tools: Vec<Box<dyn Tool>>,
}

impl Registry {
    /// The mangopi-compatible built-ins: read/write/edit/search/grep/shell/
    /// attempt_completion. `view_image` is reached via `read` on image files
    /// (and exposed here too for explicit calls).
    pub fn builtins() -> Self {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(read::ReadTool),
            Box::new(write::WriteTool),
            Box::new(edit::EditTool),
            Box::new(search::SearchTool),
            Box::new(grep::GrepTool),
            Box::new(shell::ShellTool),
            image_view_tool(),
            Box::new(plan::UpdatePlanTool),
            Box::new(attempt_completion::AttemptCompletionTool),
        ];
        Self { tools }
    }

    pub fn add(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }

    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools.iter().map(|t| t.schema()).collect()
    }

    pub fn names(&self) -> Vec<&str> {
        self.tools.iter().map(|t| t.name()).collect()
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools
            .iter()
            .find(|t| t.name() == name)
            .map(|t| t.as_ref())
    }

    /// Dispatch a tool call: enforce the approval policy, emit the call card,
    /// run `before`, then `run`.
    pub async fn run(&self, name: &str, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        let Some(tool) = self.get(name) else {
            return ToolResult::fail(format!("unknown tool '{name}'"));
        };
        // Per-tool approval gate. `Deny` blocks before the call card even shows;
        // `Ask` shows the card then routes through `confirm`; `Allow` runs as before.
        let approval = env.approval_mode(name).await;
        if approval == env::Approval::Deny {
            return ToolResult::fail(format!("tool '{name}' is blocked by the approval policy"));
        }
        let preview = tool.preview(args);
        env.emit(ToolEvent::Call {
            name: name.to_string(),
            preview,
        })
        .await;
        if approval == env::Approval::Ask && !env.confirm(&format!("Run tool '{name}'?")).await {
            env.emit(ToolEvent::Result { ok: false }).await;
            return ToolResult::fail(format!("tool '{name}' was denied by the user"));
        }
        tool.before(args, env).await;
        let result = tool.run(args, env).await;
        env.emit(ToolEvent::Result { ok: result.success }).await;
        result
    }
}

/// A thin `view_image` tool wrapper around the shared image helper.
struct ViewImageTool;
fn image_view_tool() -> Box<dyn Tool> {
    Box::new(ViewImageTool)
}

#[async_trait::async_trait]
impl Tool for ViewImageTool {
    fn name(&self) -> &str {
        "view_image"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "view_image",
            "Analyze a local image (screenshot, UI mockup, diagram, figure) with the configured vision model. Accepts an absolute path to a file on disk; URLs are not supported.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path to a local image file (png/jpg/jpeg/gif/webp)" },
                    "question": { "type": "string", "description": "Optional specific question or extraction goal for the vision model" }
                },
                "required": ["path"]
            }),
        )
    }
    fn preview(&self, args: &Value) -> String {
        args.get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }
    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => return ToolResult::fail("view_image error: 'path' is required"),
        };
        image::view_image(&path)
    }
}

#[cfg(test)]
mod approval_tests {
    use super::*;
    use crate::env::{Approval, ToolEnv, ToolEvent};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    /// A tool that flips a flag when it actually runs, so we can assert whether
    /// the approval gate let it through.
    struct SpyTool(&'static AtomicBool);
    #[async_trait::async_trait]
    impl Tool for SpyTool {
        fn name(&self) -> &str {
            "spy"
        }
        fn schema(&self) -> ToolSchema {
            ToolSchema::new("spy", "test", serde_json::json!({"type": "object"}))
        }
        async fn run(&self, _args: &Value, _env: &dyn ToolEnv) -> ToolResult {
            self.0.store(true, Ordering::SeqCst);
            ToolResult::ok("ran")
        }
    }

    struct PolicyEnv {
        root: PathBuf,
        mode: Approval,
        confirm_ok: bool,
    }
    #[async_trait::async_trait]
    impl ToolEnv for PolicyEnv {
        fn project_root(&self) -> &Path {
            &self.root
        }
        async fn confirm(&self, _message: &str) -> bool {
            self.confirm_ok
        }
        async fn approval_mode(&self, _tool: &str) -> Approval {
            self.mode
        }
        async fn emit(&self, _event: ToolEvent) {}
    }

    struct EventEnv {
        root: PathBuf,
        events: Mutex<Vec<ToolEvent>>,
    }

    #[async_trait::async_trait]
    impl ToolEnv for EventEnv {
        fn project_root(&self) -> &Path {
            &self.root
        }
        async fn confirm(&self, _message: &str) -> bool {
            true
        }
        async fn emit(&self, event: ToolEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    async fn run_with(mode: Approval, confirm_ok: bool) -> (bool, ToolResult) {
        static RAN: AtomicBool = AtomicBool::new(false);
        RAN.store(false, Ordering::SeqCst);
        let mut reg = Registry { tools: vec![] };
        reg.add(Box::new(SpyTool(&RAN)));
        let env = PolicyEnv {
            root: PathBuf::from("."),
            mode,
            confirm_ok,
        };
        let res = reg.run("spy", &serde_json::json!({}), &env).await;
        (RAN.load(Ordering::SeqCst), res)
    }

    #[tokio::test]
    async fn approval_gate() {
        // Deny: never runs, fails.
        let (ran, res) = run_with(Approval::Deny, true).await;
        assert!(!ran && !res.success, "deny must block the tool");
        // Ask + confirm no: never runs, fails.
        let (ran, res) = run_with(Approval::Ask, false).await;
        assert!(!ran && !res.success, "ask+deny must block the tool");
        // Ask + confirm yes: runs.
        let (ran, res) = run_with(Approval::Ask, true).await;
        assert!(ran && res.success, "ask+approve must run the tool");
        // Allow: runs without asking.
        let (ran, res) = run_with(Approval::Allow, false).await;
        assert!(ran && res.success, "allow must run the tool");
    }

    #[tokio::test]
    async fn shell_tool_emits_single_call_event() {
        let reg = Registry::builtins();
        let env = EventEnv {
            root: std::env::current_dir().unwrap(),
            events: Mutex::new(vec![]),
        };
        let cmd = if cfg!(target_os = "windows") {
            "Write-Output ok"
        } else {
            "printf ok"
        };

        let res = reg
            .run("shell", &serde_json::json!({ "cmd": cmd }), &env)
            .await;

        assert!(res.success, "shell command should succeed: {}", res.content);
        let calls = env
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|ev| matches!(ev, ToolEvent::Call { .. }))
            .count();
        assert_eq!(calls, 1, "registry should emit the only tool call card");
    }
}
