//! Structured lifecycle events for one agent run.
//!
//! The agent loop emits these through [`crate::Output::runtime_event`]. They
//! are host-agnostic: the Tauri shell projects them onto its own UI event
//! protocol and headless hosts can ignore them entirely. Terminology:
//!
//! - **Run**: one `agent_loop` / `agent_loop_continue` invocation.
//! - **Round**: one model request together with the tool batch it triggers.
//!
//! Tool lifecycle is deliberately split into distinct states so a host can
//! tell "the model is still typing arguments" apart from "the tool is
//! actually running":
//!
//! - [`AgentRuntimeEvent::AssistantToolCallUpdated`]: argument draft, still
//!   streaming. Live-only; never persisted.
//! - [`AgentRuntimeEvent::ToolCallReady`]: the assistant message is complete
//!   and the call's arguments are final.
//! - [`AgentRuntimeEvent::ToolExecutionStarted`] / `Updated` / `Finished`:
//!   the tool is executing.
//! - [`AgentRuntimeEvent::ToolExecutionBlocked`]: the call was rejected by a
//!   policy or user decision instead of running.
//!
//! Exactly one [`AgentRuntimeEvent::RunFinished`] is emitted per run, whether
//! the run completes, fails, is truncated, or is cancelled.

/// Stable identity of one tool invocation within a run. The draft phase keys
/// on `(round, index)` only; `call_id` is bound once the assistant message
/// finishes and the provider-assigned id is known.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolInvocationKey {
    pub round: usize,
    pub index: usize,
    pub call_id: Option<String>,
}

impl ToolInvocationKey {
    pub fn draft(round: usize, index: usize) -> Self {
        Self {
            round,
            index,
            call_id: None,
        }
    }

    pub fn ready(round: usize, index: usize, call_id: impl Into<String>) -> Self {
        Self {
            round,
            index,
            call_id: Some(call_id.into()),
        }
    }
}

/// How a run ended. `Failed` carries the user-facing error message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    Completed,
    Cancelled,
    Failed(String),
}

/// Snapshot of a tool call whose arguments are still streaming. Always a
/// full snapshot (never a delta): a retry replaces the previous snapshot for
/// the same `(round, index)` instead of appending to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallDraft {
    pub key: ToolInvocationKey,
    /// Provider-assigned call id, once the stream has revealed it.
    pub id: Option<String>,
    pub name: String,
    /// Raw arguments received so far — possibly not yet valid JSON.
    pub arguments_so_far: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentRuntimeEvent {
    RunStarted,
    RoundStarted {
        round: usize,
    },
    AssistantMessageStarted {
        round: usize,
    },
    AssistantTextDelta {
        round: usize,
        delta: String,
    },
    AssistantReasoningDelta {
        round: usize,
        delta: String,
    },
    /// A tool-call argument draft updated. Live-only: hosts must not persist
    /// `arguments_so_far` (it may be incomplete or sensitive); render only
    /// what the tool's `preview()` produces from it.
    AssistantToolCallUpdated(ToolCallDraft),
    AssistantMessageFinished {
        round: usize,
    },
    /// The assistant message is complete; this call's arguments are final.
    ToolCallReady {
        key: ToolInvocationKey,
        name: String,
    },
    ToolExecutionStarted {
        key: ToolInvocationKey,
        name: String,
    },
    /// Structured mid-execution progress. Never enters the model context.
    ToolExecutionUpdated {
        key: ToolInvocationKey,
        details: serde_json::Value,
    },
    ToolExecutionFinished {
        key: ToolInvocationKey,
        name: String,
        ok: bool,
        duration_ms: u64,
    },
    /// The call did not run: a user decision on a sibling call invalidated
    /// it, or a policy refused it.
    ToolExecutionBlocked {
        key: ToolInvocationKey,
        name: String,
        reason: String,
    },
    RoundFinished {
        round: usize,
    },
    RunFinished {
        outcome: RunOutcome,
    },
}
