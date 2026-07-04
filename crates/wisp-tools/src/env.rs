//! Tool execution environment: project root, approval, and UI event sink.

use async_trait::async_trait;
use std::path::Path;

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
    Stdout {
        chunk: String,
    },
    Result {
        ok: bool,
    },
}

/// The environment tools run in. The agent loop supplies this; the headless
/// CLI and the Tauri host each implement it.
#[async_trait]
pub trait ToolEnv: Send + Sync {
    fn project_root(&self) -> &Path;
    /// Ask the user to approve a potentially-destructive action.
    async fn confirm(&self, message: &str) -> bool;
    /// Emit a UI event (best-effort; never blocks the tool).
    async fn emit(&self, event: ToolEvent);
    /// Whether the user has requested cancellation (Stop button). Long-running
    /// tools (shell, python) poll this so a running child can be killed mid-exec
    /// instead of only between agent iterations. Default `false` for envs that
    /// don't support cancellation (e.g. tests).
    fn is_cancelled(&self) -> bool {
        false
    }
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
