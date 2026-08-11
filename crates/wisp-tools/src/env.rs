//! Tool execution environment: project root, approval, and UI event sink.

use async_trait::async_trait;
use std::path::{Path, PathBuf};

/// A host-owned resource lease held for one complete tool call.
///
/// The tools crate deliberately does not know how the desktop coordinates
/// projects or conversations. Hosts that do coordinate them return a lease
/// whose release callback removes the active claim when the tool finishes,
/// fails, or is cancelled. Headless hosts keep the default `None` behavior.
pub struct ToolResourceLease {
    release: Option<Box<dyn FnOnce() + Send + 'static>>,
}

impl ToolResourceLease {
    pub fn new(release: impl FnOnce() + Send + 'static) -> Self {
        Self {
            release: Some(Box::new(release)),
        }
    }
}

impl Drop for ToolResourceLease {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

/// Events a tool emits to the UI as it runs (tool-call card, diff preview,
/// live stdout, result tick).
#[derive(Debug, Clone)]
pub enum ToolEvent {
    Call {
        name: String,
        preview: String,
    },
    Diff {
        path: String,
        old: String,
        new: String,
    },
    /// Emitted only after a file mutation has been committed successfully.
    /// UI previews use this instead of the pre-write diff event so they never
    /// race the filesystem and reload stale content.
    FileChanged {
        path: String,
    },
    Stdout {
        chunk: String,
    },
    /// A host-renderable, non-model presentation such as an MCP App. The
    /// payload is forwarded to capable UIs but is deliberately excluded from
    /// the language-model context.
    Presentation {
        kind: String,
        payload: serde_json::Value,
    },
    /// Structured mid-execution progress for the host/UI (e.g. a live Run
    /// snapshot while a monitor tool waits). Never enters the model context;
    /// the agent loop forwards it only while the emitting invocation is
    /// still active and drops anything emitted after the tool finished.
    Progress {
        details: serde_json::Value,
    },
    Result {
        ok: bool,
    },
}

/// Per-tool approval policy, applied by `Registry::run` before a tool executes.
/// `Allow` runs silently (the default — preserves the old auto-run behaviour);
/// `Ask` routes through `confirm`; `Deny` blocks the call outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmDecision {
    Approved,
    Denied { feedback: Option<String> },
}

impl ConfirmDecision {
    pub fn approved(&self) -> bool {
        matches!(self, Self::Approved)
    }

    pub fn feedback(&self) -> Option<&str> {
        match self {
            Self::Denied {
                feedback: Some(feedback),
            } => {
                let trimmed = feedback.trim();
                (!trimmed.is_empty()).then_some(trimmed)
            }
            _ => None,
        }
    }
}

/// The environment tools run in. The agent loop supplies this; the headless
/// CLI and the Tauri host each implement it.
#[async_trait]
pub trait ToolEnv: Send + Sync {
    fn project_root(&self) -> &Path;
    /// Restrict read/search paths to the project root. Main Agents keep the
    /// legacy unrestricted default; delegated environments opt in.
    fn restrict_read_paths_to_project(&self) -> bool {
        false
    }
    fn resolve_read_path(&self, path: &str, allow_directory: bool) -> Result<PathBuf, String> {
        if let Some(id) = path.strip_prefix("wisp-history:") {
            let stem = id.strip_suffix(".json").unwrap_or(id);
            if stem.is_empty()
                || stem.len() > 128
                || !stem
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                return Err("invalid wisp-history reference".into());
            }
            let filename = if id.ends_with(".json") {
                id.to_string()
            } else {
                format!("{id}.json")
            };
            return Ok(self
                .project_root()
                .join(".wisp")
                .join("history")
                .join(filename));
        }
        if !self.restrict_read_paths_to_project() {
            return Ok(PathBuf::from(path));
        }
        if allow_directory {
            crate::safety::resolve_under_root(self.project_root(), path)
        } else {
            crate::safety::validate_file_path(self.project_root(), path)
        }
    }
    /// Ask the user to approve a potentially-destructive action.
    async fn confirm(&self, message: &str) -> bool;
    /// Ask the user to approve an action, optionally carrying rejection feedback.
    async fn confirm_decision(&self, message: &str) -> ConfirmDecision {
        if self.confirm(message).await {
            ConfirmDecision::Approved
        } else {
            ConfirmDecision::Denied { feedback: None }
        }
    }
    /// Approval mode for a tool about to run. Default `Allow` keeps the CLI and
    /// tests auto-running; the Tauri host overrides this from its saved policy.
    async fn approval_mode(&self, _tool: &str) -> Approval {
        Approval::Allow
    }
    /// Acquire any cross-conversation resources needed by this call. The
    /// registry holds the returned lease across `before` and `run`, so a host
    /// can make a read-modify-write tool one indivisible coordinated action.
    async fn acquire_tool_resources(
        &self,
        _tool: &str,
        _args: &serde_json::Value,
    ) -> Result<Option<ToolResourceLease>, String> {
        Ok(None)
    }
    /// Whether approval prompts should be bypassed for this conversation.
    /// Explicit host `Deny` rules and hard safety boundaries still win. This is
    /// stronger than returning [`Approval::Allow`]: it also suppresses a
    /// tool's built-in minimum approval requirement.
    fn approval_bypass(&self) -> bool {
        false
    }
    /// Whether this session is in plan mode: the agent researches and drafts a
    /// plan, so `Registry::run` refuses every tool outside
    /// [`crate::PLAN_MODE_READ_ONLY`]. Default `false`.
    fn plan_mode(&self) -> bool {
        false
    }
    /// Whether project mutations are temporarily locked while this
    /// conversation remains available for read-only work. This is distinct
    /// from plan mode: the user may ask for a normal answer, but mutating tools
    /// must fail closed.
    fn project_write_locked(&self) -> bool {
        false
    }
    /// Whether the "full" approval scope is active — auto-approve everything,
    /// dangerous commands included. Only the shell danger check consults this;
    /// default `false` keeps the CLI and tests prompting on dangerous commands.
    fn danger_auto_approve(&self) -> bool {
        false
    }
    /// Emit a UI event (best-effort; never blocks the tool).
    async fn emit(&self, event: ToolEvent);
    /// Whether the user has requested cancellation (Stop button). Long-running
    /// tools (shell, python) poll this so a running child can be killed mid-exec
    /// instead of only between agent iterations. Default `false` for envs that
    /// don't support cancellation (e.g. tests).
    fn is_cancelled(&self) -> bool {
        false
    }
    /// The raw cancel flag, when the env has one. Lets a tool that runs a
    /// nested agent loop (subagents) pass the SAME Stop flag through, so the
    /// user's Stop also interrupts the inner loop. Default `None`.
    fn cancel_flag(&self) -> Option<&std::sync::atomic::AtomicBool> {
        None
    }
    /// Optional pre-check before spawning a shell command (e.g. block free-form
    /// SSH against a host the app already failed to reach). Default allows all.
    async fn preflight_shell(&self, _cmd: &str) -> Result<(), String> {
        Ok(())
    }
    /// Optional post-check after a shell command finishes so the host can open
    /// a connectivity gate without spawning another SSH attempt.
    fn note_shell_outcome(&self, _cmd: &str, _success: bool, _detail: &str) {}
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub success: bool,
    pub content: String,
    pub image: Option<ImageData>,
    /// Structured result data for the host/UI (e.g. a Run record). `content`
    /// is the model-facing summary; `details` must NEVER enter the model
    /// context — the agent loop forwards it on the tool-finished runtime
    /// event instead of appending it to the conversation.
    pub details: Option<serde_json::Value>,
    /// Code-level control flow for the agent loop. This keeps user-decision
    /// boundaries out of prompt wording: stale sibling calls can be skipped,
    /// and tools such as `ask_user` can end the turn outright.
    pub control: ToolControl,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ToolControl {
    #[default]
    Continue,
    StopBatch,
    StopTurn,
}

#[derive(Debug, Clone)]
pub struct ImageData {
    pub mime: String,
    /// A `data:` URI ready for an OpenAI-compatible `image_url` part.
    pub data_url: String,
    pub label: String,
}

impl ToolResult {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            success: true,
            content: content.into(),
            image: None,
            details: None,
            control: ToolControl::Continue,
        }
    }
    pub fn fail(content: impl Into<String>) -> Self {
        Self {
            success: false,
            content: content.into(),
            image: None,
            details: None,
            control: ToolControl::Continue,
        }
    }
    pub fn image(img: ImageData) -> Self {
        let label = img.label.clone();
        Self {
            success: true,
            content: label,
            image: Some(img),
            details: None,
            control: ToolControl::Continue,
        }
    }
    /// Attach structured host/UI details. Never reaches the model context.
    pub fn with_details(mut self, details: impl Into<serde_json::Value>) -> Self {
        self.details = Some(details.into());
        self
    }
    /// Skip tool calls that the model placed later in the same batch, then let
    /// the model react to this result in a fresh iteration.
    pub fn stop_batch(mut self) -> Self {
        self.control = ToolControl::StopBatch;
        self
    }
    /// Skip later calls in the batch and return control to the user without
    /// issuing another model request.
    pub fn stop_turn(mut self) -> Self {
        self.control = ToolControl::StopTurn;
        self
    }
}
