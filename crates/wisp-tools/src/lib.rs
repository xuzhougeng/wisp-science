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
pub mod process;
pub mod read;
pub mod safety;
pub mod search;
pub mod shell;
pub mod tool;
pub mod write;

pub use env::{ImageData, ToolEnv, ToolEvent, ToolResult};
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

    /// Dispatch a tool call: emit the call card, run `before`, then `run`.
    pub async fn run(&self, name: &str, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        let Some(tool) = self.get(name) else {
            return ToolResult::fail(format!("unknown tool '{name}'"));
        };
        let preview = tool.preview(args);
        env.emit(ToolEvent::Call {
            name: name.to_string(),
            preview,
        })
        .await;
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
            "Load a local image (screenshot, UI mockup, diagram) into the model's vision context. Accepts an absolute path to a file on disk; URLs are not supported.",
            serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string", "description": "Absolute path to a local image file (png/jpg/jpeg/gif/webp)" } },
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
