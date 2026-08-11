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

/// Serialized-size cap for structured tool details/progress payloads. Tool
/// output is an unbounded external payload; details forwarded to hosts must
/// stay small enough to persist and stream per tool call.
pub const TOOL_DETAILS_MAX_BYTES: usize = 16 * 1024;

/// Bound one structured details payload for event forwarding. Values whose
/// serialized form exceeds [`TOOL_DETAILS_MAX_BYTES`] are replaced by a small
/// marker object — large details must be capped (or converted to a reference
/// by the tool itself), never forwarded unbounded.
pub fn bound_tool_details(details: serde_json::Value) -> serde_json::Value {
    let serialized = serde_json::to_string(&details).unwrap_or_default();
    if serialized.len() <= TOOL_DETAILS_MAX_BYTES {
        return details;
    }
    serde_json::json!({
        "truncated": true,
        "original_bytes": serialized.len(),
    })
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
        /// The tool's final structured details (bounded via
        /// [`bound_tool_details`]). Host/UI only; never enters the model
        /// context.
        details: Option<serde_json::Value>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_details_pass_through_untouched() {
        let details = serde_json::json!({"run_id": "r1", "status": "running"});
        assert_eq!(bound_tool_details(details.clone()), details);
    }

    #[test]
    fn oversized_details_collapse_to_a_marker() {
        let details = serde_json::json!({"blob": "x".repeat(TOOL_DETAILS_MAX_BYTES + 1)});
        let bounded = bound_tool_details(details);
        assert_eq!(bounded["truncated"], serde_json::json!(true));
        assert!(bounded["original_bytes"].as_u64().unwrap() as usize > TOOL_DETAILS_MAX_BYTES);
        assert!(serde_json::to_string(&bounded).unwrap().len() <= TOOL_DETAILS_MAX_BYTES);
    }
}
