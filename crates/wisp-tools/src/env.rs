//! Tool execution environment: project root, approval, and UI event sink.

use async_trait::async_trait;
use std::path::{Path, PathBuf};

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
        }
    }
    pub fn fail(content: impl Into<String>) -> Self {
        Self {
            success: false,
            content: content.into(),
            image: None,
        }
    }
    pub fn image(img: ImageData) -> Self {
        let label = img.label.clone();
        Self {
            success: true,
            content: label,
            image: Some(img),
        }
    }
}
