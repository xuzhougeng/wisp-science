//! Tool execution environment: project root, approval, and UI event sink.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
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

/// A stable, domain-specific explanation of a write before it is committed.
///
/// Generic tool approval is insufficient for physical lab operations: the user
/// needs to see identities, quantities, assumptions, and missing information.
/// Hosts may render this as a rich card; the text representation keeps CLI and
/// older hosts deterministic and safe.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainConfirmationRequest {
    pub domain: String,
    pub command_id: String,
    #[serde(default)]
    pub transaction_id: Option<String>,
    #[serde(default)]
    pub affected_ids: Vec<String>,
    #[serde(default)]
    pub before: serde_json::Value,
    #[serde(default)]
    pub after: serde_json::Value,
    pub risk_class: String,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default)]
    pub missing_data: Vec<String>,
    #[serde(default)]
    pub actions: Vec<String>,
}

impl DomainConfirmationRequest {
    pub fn text_fallback(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| {
            format!(
                "Confirm {} command {} (risk: {})",
                self.domain, self.command_id, self.risk_class
            )
        })
    }
}

/// The environment tools run in. The agent loop supplies this; the headless
/// CLI and the Tauri host each implement it.
#[async_trait]
pub trait ToolEnv: Send + Sync {
    fn project_root(&self) -> &Path;
    /// Domain-owned projections that generic write/edit tools must not mutate.
    fn is_write_path_protected(&self, _path: &Path) -> bool {
        false
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
    /// Request confirmation of a structured domain mutation. The default keeps
    /// existing headless hosts compatible by using a deterministic text card.
    async fn confirm_domain(&self, request: &DomainConfirmationRequest) -> ConfirmDecision {
        self.confirm_decision(&request.text_fallback()).await
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
