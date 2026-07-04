//! UI/CLI output abstraction. The agent loop drives this; the headless CLI
//! prints to the terminal and the Tauri host forwards each call as an event.
//!
//! All methods take `&self` so a single shared `Output` can be borrowed by the
//! tool environment and the stream sink simultaneously. Interactive state
//! (confirmation prompts) is guarded with interior mutability in impls.

use serde_json::Value;

pub trait Output: Send + Sync {
    fn assistant_text(&self, _delta: &str) {}
    fn reasoning(&self, _delta: &str) {}
    fn tool_call(&self, _name: &str, _preview: &str) {}
    fn tool_result(&self, _name: &str, _ok: bool, _content: &str) {}
    fn usage(
        &self,
        _round: usize,
        _input: u64,
        _output: u64,
        _ctx_tokens: usize,
        _max_context: usize,
    ) {
    }
    fn compaction(&self, _before: usize, _after: usize, _strategy: &str) {}
    fn diff(&self, _path: &str, _old: &str, _new: &str) {}
    fn stdout_chunk(&self, _chunk: &str) {}
    /// Blocking confirmation prompt for destructive actions.
    fn confirm(&self, _message: &str) -> bool {
        true
    }
    /// Approval mode for a tool about to run. Default `Allow` preserves the old
    /// auto-run behaviour; the Tauri host overrides it from its saved policy.
    fn approval_mode(&self, _tool: &str) -> wisp_tools::Approval {
        wisp_tools::Approval::Allow
    }
    /// Fired once per message appended to the context during a turn (user,
    /// assistant, tool). Lets the host persist incrementally so a crash or a
    /// mid-turn "new session" doesn't lose the whole turn. Default: no-op.
    fn on_message(&self, _msg: &wisp_llm::Message) {}
}

/// A silent output for tests / non-interactive runs that auto-approves.
pub struct NullOutput;
impl Output for NullOutput {}

/// Adapter exposing `Output` as a `wisp_tools::ToolEnv`.
pub struct ToolEnvAdapter<'a> {
    root: std::path::PathBuf,
    out: &'a dyn Output,
    cancel: Option<&'a std::sync::atomic::AtomicBool>,
}

impl<'a> ToolEnvAdapter<'a> {
    pub fn new(root: std::path::PathBuf, out: &'a dyn Output) -> Self {
        Self {
            root,
            out,
            cancel: None,
        }
    }
    /// Like `new`, but tools can poll `is_cancelled()` to stop mid-execution.
    pub fn with_cancel(
        root: std::path::PathBuf,
        out: &'a dyn Output,
        cancel: &'a std::sync::atomic::AtomicBool,
    ) -> Self {
        Self {
            root,
            out,
            cancel: Some(cancel),
        }
    }
}

#[async_trait::async_trait]
impl<'a> wisp_tools::ToolEnv for ToolEnvAdapter<'a> {
    fn project_root(&self) -> &std::path::Path {
        &self.root
    }
    async fn confirm(&self, message: &str) -> bool {
        self.out.confirm(message)
    }
    async fn approval_mode(&self, tool: &str) -> wisp_tools::Approval {
        self.out.approval_mode(tool)
    }
    fn is_cancelled(&self) -> bool {
        self.cancel
            .is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
    }
    async fn emit(&self, event: wisp_tools::ToolEvent) {
        match event {
            wisp_tools::ToolEvent::Call { name, preview } => self.out.tool_call(&name, &preview),
            wisp_tools::ToolEvent::Diff { path, old, new } => self.out.diff(&path, &old, &new),
            wisp_tools::ToolEvent::Stdout { chunk } => self.out.stdout_chunk(&chunk),
            wisp_tools::ToolEvent::Result { ok: _ } => {}
        }
        let _ = Value::Null;
    }
}

/// Adapter exposing `Output` as a `wisp_llm::StreamSink` (text + reasoning
/// deltas only; usage/tool-call deltas are handled by the agent loop).
pub struct StreamSinkAdapter<'a> {
    out: &'a dyn Output,
}
impl<'a> StreamSinkAdapter<'a> {
    pub fn new(out: &'a dyn Output) -> Self {
        Self { out }
    }
}
impl<'a> wisp_llm::StreamSink for StreamSinkAdapter<'a> {
    fn on_text(&mut self, delta: &str) {
        self.out.assistant_text(delta);
    }
    fn on_reasoning(&mut self, delta: &str) {
        self.out.reasoning(delta);
    }
    fn on_tool_call(&mut self, _i: usize, _name: &str, _args: &str) {}
    fn on_usage(&mut self, _u: wisp_llm::Usage) {}
}
