//! Data model for the UI: the serde DTOs exchanged with the Tauri backend plus
//! the in-memory view/form types.
//!
//! This module holds *data only* — struct/enum shapes and trivial inherent
//! impls (defaults, conversions, small classifiers). It must not depend on
//! Leptos reactivity, the JS bindings, or view code, so the shapes stay easy to
//! reason about and reuse. Fields are `pub(crate)` so the rest of the crate can
//! read/build them; behaviour that needs i18n, signals, or FFI lives elsewhere.

use crate::i18n::Locale;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Deserialize, Serialize, Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub(crate) struct ContextUsage {
    #[serde(default)]
    pub(crate) system_prompt: usize,
    #[serde(default)]
    pub(crate) tool_definitions: usize,
    #[serde(default)]
    pub(crate) rules: usize,
    #[serde(default)]
    pub(crate) skills: usize,
    #[serde(default)]
    pub(crate) mcp_dynamic_tools: usize,
    #[serde(default)]
    pub(crate) subagent_definitions: usize,
    #[serde(default)]
    pub(crate) conversation: usize,
}

impl ContextUsage {
    pub(crate) fn total(self) -> usize {
        self.system_prompt
            .saturating_add(self.tool_definitions)
            .saturating_add(self.rules)
            .saturating_add(self.skills)
            .saturating_add(self.mcp_dynamic_tools)
            .saturating_add(self.subagent_definitions)
            .saturating_add(self.conversation)
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) struct ContextUsageSnapshot {
    pub(crate) used: usize,
    pub(crate) max: usize,
    pub(crate) breakdown: Option<ContextUsage>,
    pub(crate) estimated: bool,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ContextToolDetail {
    pub(crate) name: String,
    pub(crate) description: String,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ContextUsageDetails {
    #[serde(default)]
    pub(crate) system_prompt: String,
    #[serde(default)]
    pub(crate) tool_definitions: Vec<ContextToolDetail>,
    #[serde(default)]
    pub(crate) rules: String,
    #[serde(default)]
    pub(crate) skills: String,
    #[serde(default)]
    pub(crate) mcp_dynamic_tools: Vec<ContextToolDetail>,
    #[serde(default)]
    pub(crate) subagent_definitions: Vec<ContextToolDetail>,
}

/// Progress emitted by the native project archive importer/exporter. Mirrors
/// `ProjectTransferProgress` in `src-tauri/src/project_transfer.rs`.
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectTransferProgress {
    pub(crate) direction: String,
    pub(crate) stage: String,
    #[serde(default)]
    pub(crate) project_id: Option<String>,
    pub(crate) completed_files: u64,
    pub(crate) total_files: Option<u64>,
    pub(crate) completed_bytes: u64,
    pub(crate) total_bytes: Option<u64>,
    #[serde(default)]
    pub(crate) current_path: Option<String>,
    #[serde(default)]
    pub(crate) error: Option<String>,
}

impl ProjectTransferProgress {
    pub(crate) fn selecting(direction: &str, project_id: Option<String>) -> Self {
        Self {
            direction: direction.into(),
            stage: if direction == "export" {
                "selecting_export_destination".into()
            } else {
                "selecting_archive".into()
            },
            project_id,
            completed_files: 0,
            total_files: None,
            completed_bytes: 0,
            total_bytes: None,
            current_path: None,
            error: None,
        }
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.stage == "complete"
    }

    pub(crate) fn is_failed(&self) -> bool {
        self.stage == "failed"
    }

    pub(crate) fn is_active(&self) -> bool {
        !self.is_complete() && !self.is_failed()
    }

    pub(crate) fn is_exporting_project(&self, project_id: &str) -> bool {
        self.is_active()
            && self.direction == "export"
            && self.project_id.as_deref() == Some(project_id)
    }

    pub(crate) fn complete(
        direction: &str,
        project_id: Option<String>,
        current_path: Option<String>,
    ) -> Self {
        Self {
            direction: direction.into(),
            stage: "complete".into(),
            project_id,
            completed_files: 0,
            total_files: None,
            completed_bytes: 0,
            total_bytes: None,
            current_path,
            error: None,
        }
    }

    pub(crate) fn failed(
        direction: &str,
        project_id: Option<String>,
        error: String,
    ) -> Self {
        Self {
            direction: direction.into(),
            stage: "failed".into(),
            project_id,
            completed_files: 0,
            total_files: None,
            completed_bytes: 0,
            total_bytes: None,
            current_path: None,
            error: Some(error),
        }
    }
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CustomCredentialStatus {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) env_var: String,
    pub(crate) present: bool,
}

#[derive(Deserialize, Clone, Debug, Hash, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MessageResource {
    pub(crate) id: String,
    pub(crate) ordinal: i64,
    pub(crate) original_reference: String,
    pub(crate) artifact_id: Option<String>,
    pub(crate) artifact_version_id: Option<String>,
    pub(crate) display_name: String,
    pub(crate) kind: String,
    pub(crate) mime_type: String,
    pub(crate) status: String,
    pub(crate) error: Option<String>,
}

#[derive(Deserialize, Clone)]
#[serde(tag = "kind")]
pub(crate) enum AgentEvent {
    User {
        frame_id: String,
        text: String,
    },
    MessageBoundary {
        frame_id: String,
        seq: i64,
    },
    Resources {
        frame_id: String,
        resources: Vec<MessageResource>,
    },
    Text {
        frame_id: String,
        delta: String,
    },
    Reasoning {
        frame_id: String,
        delta: String,
    },
    ToolCall {
        frame_id: String,
        name: String,
        preview: String,
    },
    /// Live-only draft of a tool call whose arguments are still streaming.
    /// Keyed by `call_key` (`{round}:{index}`); superseded by the real
    /// ToolCall or dropped by ToolCallDraftClear. Never persisted host-side.
    ToolCallDraft {
        frame_id: String,
        call_key: String,
        name: String,
        preview: String,
    },
    /// Live-only: clear one draft (`Some(key)`) or all drafts of the frame
    /// (`None`) so no ghost rows remain.
    ToolCallDraftClear {
        frame_id: String,
        #[serde(default)]
        call_key: Option<String>,
    },
    /// Live-only: the tool started executing; stamps the invocation key onto
    /// the pending tool row so progress/details match by `call_key`.
    ToolExecutionStarted {
        frame_id: String,
        call_key: String,
        name: String,
    },
    /// Live-only structured mid-execution progress; never persisted host-side.
    ToolProgress {
        frame_id: String,
        call_key: String,
        details: serde_json::Value,
    },
    /// Persisted patch with a tool's final structured details; applied to the
    /// matching tool row (by `call_key`, falling back to name).
    ToolResultDetails {
        frame_id: String,
        call_key: String,
        name: String,
        details: serde_json::Value,
    },
    ToolResult {
        frame_id: String,
        name: String,
        ok: bool,
        content: String,
        #[serde(default)]
        duration_ms: u64,
    },
    ToolPresentation {
        frame_id: String,
        #[serde(default)]
        presentation_id: String,
        presentation_kind: String,
        payload: serde_json::Value,
    },
    Usage {
        frame_id: String,
        input: u64,
        output: u64,
        #[serde(default)]
        reasoning: u64,
        #[serde(default)]
        cached: u64,
        ctx_tokens: usize,
        max_context: usize,
        #[serde(default)]
        context_usage: ContextUsage,
    },
    Compaction {
        frame_id: String,
        before: usize,
        after: usize,
        strategy: String,
    },
    CompactionStarted {
        frame_id: String,
    },
    ContextWarning {
        frame_id: String,
        ctx_tokens: usize,
        max_context: usize,
    },
    Diff {},
    FileChanged {
        frame_id: String,
        path: String,
    },
    Stdout {
        frame_id: String,
        chunk: String,
    },
    Done {
        frame_id: String,
        #[serde(default)]
        stop_reason: Option<String>,
    },
    Error {
        frame_id: String,
        message: String,
    },
    DelegationCompleted {
        frame_id: String,
        workflow_id: String,
        status: String,
        result: String,
        auto_resume: bool,
    },
    ReviewStarted {
        frame_id: String,
    },
    ReviewFailed {
        frame_id: String,
        message: String,
    },
    Review {
        frame_id: String,
        report: ReviewReport,
    },
    CorrectionStarted {
        frame_id: String,
        model: String,
    },
}

#[derive(Deserialize, Clone, Hash, PartialEq, Eq)]
pub(crate) struct ReviewFinding {
    #[serde(default)]
    pub(crate) message_index: usize,
    #[serde(default)]
    pub(crate) claim: String,
    #[serde(default)]
    pub(crate) evidence: String,
    #[serde(default)]
    pub(crate) fix: String,
    #[serde(default)]
    pub(crate) verdict: String,
    #[serde(default)]
    pub(crate) severity: String,
    #[serde(default)]
    pub(crate) status: String,
}

#[derive(Deserialize, Clone, Hash, PartialEq, Eq)]
pub(crate) struct ReviewReport {
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) summary: String,
    #[serde(default)]
    pub(crate) findings: Vec<ReviewFinding>,
    #[serde(default)]
    pub(crate) reviewer_model: String,
    #[serde(default)]
    pub(crate) reviewer_effort: String,
    #[serde(default)]
    pub(crate) reviewer_backend: String,
    #[serde(default)]
    pub(crate) review_status: String,
    #[serde(default = "default_evidence_coverage")]
    pub(crate) evidence_coverage: u8,
    #[serde(default)]
    pub(crate) coverage_gaps: Vec<String>,
}

fn default_evidence_coverage() -> u8 {
    100
}

#[derive(Clone)]
pub(crate) enum ChatItem {
    User(String),
    /// A user turn queued (#433) while the same session is still running. It
    /// waits, editable/cancellable, until the backend drains it into a fresh
    /// turn (or a cut-in folds it into the running one) and emits the matching
    /// User event. `id` is the frontend-assigned key the queue commands target.
    QueuedUser {
        id: u64,
        text: String,
    },
    Assistant {
        text: String,
        model: Option<String>,
        resources: Vec<MessageResource>,
    },
    Reasoning(String),
    Tool {
        name: String,
        ok: Option<bool>,
        input: String,
        output: String,
        /// Wall-clock start (ms) while the tool is running; cleared on result.
        started_at_ms: Option<u64>,
        /// Elapsed ms from tool call card to result.
        duration_ms: Option<u64>,
        /// Invocation identity (`{round}:{index}`). Draft rows (arguments
        /// still streaming) always carry it; live rows are stamped with it
        /// when execution starts so progress/details patches match by key.
        /// `None` on replayed rows — persisted ToolCall events have no key.
        call_key: Option<String>,
        /// Structured final details (or live progress snapshot) for the
        /// host/UI — e.g. a Run record. Never model-facing content.
        details: Option<serde_json::Value>,
    },
    /// Structured evidence that the active tool wrote a workspace file.
    /// Kept as a hidden transcript row so artifact attribution survives the
    /// persisted AgentEvent replay without scraping paths from tool output.
    FileChanged(String),
    /// Inline tool-approval card (replaces the old centered modal).
    ApprovalPending {
        tool: String,
        preview: String,
        message: String,
    },
    AcpPermission {
        request_id: String,
        tool: String,
        options: Vec<AcpPermissionOption>,
    },
    AcpTool {
        call_id: String,
        title: String,
        kind: String,
        status: String,
        content: String,
        locations: String,
    },
    /// Per-round token usage, inserted right under the assistant bubble it
    /// belongs to. Persisted per turn and rehydrated on session reload.
    Usage {
        input: u64,
        output: u64,
        reasoning: u64,
        cached: u64,
        ctx_tokens: usize,
        max_context: usize,
        context_usage: ContextUsage,
    },
    /// Persistent timeline marker emitted whenever the model context is
    /// rewritten. `strategy == "auto"` distinguishes the default 80%
    /// threshold path from an explicit `/compact`.
    Compaction {
        before: usize,
        after: usize,
        strategy: String,
    },
    /// A visible handoff between the main agent and the independent reviewer.
    ReviewTransition {
        phase: ReviewTransitionPhase,
        model: Option<String>,
    },
    Review(ReviewReport),
    Plan(PlanCard),
    Question(QuestionCard),
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SideChatEvidence {
    pub(crate) source_id: String,
    #[serde(default)]
    pub(crate) event_seq: Option<i64>,
    #[serde(default)]
    pub(crate) message_seq: Option<i64>,
    pub(crate) turn: usize,
    pub(crate) role: String,
    pub(crate) excerpt: String,
    #[serde(default)]
    pub(crate) relevance: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SideChatResponse {
    pub(crate) answer: String,
    pub(crate) snapshot_version: i64,
    #[serde(default)]
    pub(crate) evidence: Vec<SideChatEvidence>,
    #[serde(default)]
    pub(crate) no_evidence: bool,
}

#[derive(Clone)]
pub(crate) enum SideChatItem {
    User(String),
    Assistant {
        text: String,
        model: Option<String>,
        evidence: Vec<SideChatEvidence>,
        snapshot_version: i64,
        no_evidence: bool,
        error: bool,
    },
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnUndoPreview {
    #[serde(default)]
    pub(crate) restore_files: Vec<String>,
    #[serde(default)]
    pub(crate) remove_files: Vec<String>,
    #[serde(default)]
    pub(crate) remove_artifacts: Vec<String>,
    #[serde(default)]
    pub(crate) unsupported_files: Vec<String>,
    #[serde(default)]
    pub(crate) conflicts: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct TurnUndoDialog {
    pub(crate) session_id: String,
    pub(crate) user_index: usize,
    pub(crate) user_ui_index: usize,
    pub(crate) draft: String,
    pub(crate) preview: TurnUndoPreview,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum ReviewTransitionPhase {
    Reviewing,
    Correcting,
    Passed,
}

/// Hashes a string in O(1): short strings hash whole, long ones hash their
/// length plus a head/tail sample. Full-text hashing made every streaming
/// flush re-hash megabytes of transcript in long sessions, saturating the
/// WebView main thread. Streaming appends always change the tail sample, so
/// row keys still track content; a same-length edit confined to the middle of
/// a long message could miss a rebuild, which we accept for the O(1) cost.
fn hash_text_sampled<H: std::hash::Hasher>(h: &mut H, s: &str) {
    use std::hash::Hash;
    const SAMPLE: usize = 128;
    let bytes = s.as_bytes();
    bytes.len().hash(h);
    if bytes.len() <= 2 * SAMPLE {
        bytes.hash(h);
    } else {
        bytes[..SAMPLE].hash(h);
        bytes[bytes.len() - SAMPLE..].hash(h);
    }
}

impl ChatItem {
    /// Content hash used as the keyed-list key in the chat thread: a row is
    /// rebuilt only when this changes, so streaming updates to one message
    /// don't re-render the whole conversation.
    pub(crate) fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        match self {
            Self::User(s) => {
                0u8.hash(&mut h);
                hash_text_sampled(&mut h, s);
            }
            Self::QueuedUser { id, text } => {
                (1u8, id).hash(&mut h);
                hash_text_sampled(&mut h, text);
            }
            Self::Assistant {
                text,
                model,
                resources,
            } => {
                (2u8, model, resources).hash(&mut h);
                hash_text_sampled(&mut h, text);
            }
            Self::Reasoning(s) => {
                3u8.hash(&mut h);
                hash_text_sampled(&mut h, s);
            }
            Self::Tool {
                name,
                ok,
                input,
                output,
                duration_ms,
                details,
                ..
            } => {
                (4u8, name, ok, duration_ms).hash(&mut h);
                hash_text_sampled(&mut h, input);
                hash_text_sampled(&mut h, output);
                if let Some(details) = details {
                    hash_text_sampled(&mut h, &details.to_string());
                }
                details.is_some().hash(&mut h);
            }
            Self::FileChanged(path) => {
                14u8.hash(&mut h);
                hash_text_sampled(&mut h, path);
            }
            Self::ApprovalPending {
                tool,
                preview,
                message,
            } => {
                (6u8, tool).hash(&mut h);
                hash_text_sampled(&mut h, preview);
                hash_text_sampled(&mut h, message);
            }
            Self::AcpPermission {
                request_id,
                tool,
                options,
            } => (9u8, request_id, tool, options).hash(&mut h),
            Self::AcpTool {
                call_id,
                title,
                kind,
                status,
                content,
                locations,
            } => {
                (10u8, call_id, title, kind, status).hash(&mut h);
                hash_text_sampled(&mut h, content);
                hash_text_sampled(&mut h, locations);
            }
            Self::Usage {
                input,
                output,
                reasoning,
                cached,
                ctx_tokens,
                max_context,
                context_usage,
            } => (
                8u8,
                input,
                output,
                reasoning,
                cached,
                ctx_tokens,
                max_context,
                context_usage,
            )
                .hash(&mut h),
            Self::Compaction {
                before,
                after,
                strategy,
            } => (13u8, before, after, strategy).hash(&mut h),
            Self::ReviewTransition { phase, model } => (11u8, phase, model).hash(&mut h),
            Self::Review(report) => (5u8, report).hash(&mut h),
            Self::Plan(plan) => (7u8, plan).hash(&mut h),
            Self::Question(question) => (12u8, question).hash(&mut h),
        }
        h.finish()
    }
}

#[cfg(test)]
mod fingerprint_tests {
    use super::*;

    fn assistant(text: String) -> ChatItem {
        ChatItem::Assistant {
            text,
            model: None,
            resources: Vec::new(),
        }
    }

    #[test]
    fn same_content_same_fingerprint() {
        let text = "a".repeat(1000);
        assert_eq!(
            assistant(text.clone()).fingerprint(),
            assistant(text).fingerprint()
        );
    }

    #[test]
    fn streaming_append_changes_fingerprint() {
        let base = "partial answer ".repeat(100);
        let mut appended = base.clone();
        appended.push_str("next delta");
        assert_ne!(
            assistant(base).fingerprint(),
            assistant(appended).fingerprint()
        );
    }

    #[test]
    fn short_text_edit_changes_fingerprint() {
        assert_ne!(
            ChatItem::User("run the analysis".into()).fingerprint(),
            ChatItem::User("run the other analysis".into()).fingerprint()
        );
    }

    #[test]
    fn long_text_head_or_tail_edit_changes_fingerprint() {
        let middle = "x".repeat(1000);
        let head_edit = format!("EDIT{middle}");
        let tail_edit = format!("{middle}EDIT");
        assert_ne!(
            ChatItem::Reasoning(middle.clone()).fingerprint(),
            ChatItem::Reasoning(head_edit).fingerprint()
        );
        assert_ne!(
            ChatItem::Reasoning(middle.clone()).fingerprint(),
            ChatItem::Reasoning(tail_edit).fingerprint()
        );
    }

    /// Accepted tradeoff of sampled hashing: a same-length edit confined to
    /// the middle of a long message keeps the old fingerprint, so a keyed row
    /// would not rebuild. Streaming appends and typical edits always touch the
    /// length, head, or tail, so this does not occur in practice.
    #[test]
    fn middle_only_same_length_edit_is_not_detected() {
        let mut a = "y".repeat(1000);
        let mut b = a.clone();
        a.replace_range(500..503, "foo");
        b.replace_range(500..503, "bar");
        assert_eq!(
            ChatItem::User(a).fingerprint(),
            ChatItem::User(b).fingerprint()
        );
    }
}

/// One checklist row of a plan. Mirrors the ACP `plan` update entry shape,
/// which is also what Wisp persists, so one parser serves both.
#[derive(Serialize, Deserialize, Clone, Debug, Default, Hash, PartialEq, Eq)]
pub(crate) struct PlanEntry {
    #[serde(default)]
    pub(crate) content: String,
    #[serde(default)]
    pub(crate) status: PlanStatus,
    #[serde(default)]
    pub(crate) priority: PlanPriority,
}

/// `from = "String"` makes deserialization total: an agent that invents a
/// status ("blocked", "skipped") degrades to the default instead of failing the
/// whole card. Serialization still writes the ACP spelling.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
#[serde(rename_all = "snake_case", from = "String")]
pub(crate) enum PlanStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
}

impl From<String> for PlanStatus {
    fn from(raw: String) -> Self {
        match raw.as_str() {
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            _ => Self::Pending,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
#[serde(rename_all = "snake_case", from = "String")]
pub(crate) enum PlanPriority {
    Low,
    #[default]
    Medium,
    High,
}

impl From<String> for PlanPriority {
    fn from(raw: String) -> Self {
        match raw.as_str() {
            "low" => Self::Low,
            "high" => Self::High,
            _ => Self::Medium,
        }
    }
}

/// The built-in plan tool. Its result is the plan card's body, so the tool
/// event never renders as an ordinary tool row (see the `ToolResult` handler).
pub(crate) const PROPOSE_PLAN_TOOL: &str = "propose_plan";

/// Who produced the plan: the ACP agent's own plan updates, or the built-in
/// `propose_plan` tool. Both render through the same card.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
#[serde(rename_all = "snake_case", from = "String")]
pub(crate) enum PlanSource {
    Native,
    #[default]
    Acp,
}

impl From<String> for PlanSource {
    fn from(raw: String) -> Self {
        match raw.as_str() {
            "native" => Self::Native,
            _ => Self::Acp,
        }
    }
}

/// Card-level lifecycle: a plan that is still being revised this turn vs. one
/// the turn finished with. Never persisted — reloaded plans are always ready.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub(crate) enum PlanState {
    Streaming,
    #[default]
    Ready,
}

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq)]
pub(crate) struct PlanCard {
    pub(crate) entries: Vec<PlanEntry>,
    pub(crate) source: PlanSource,
    pub(crate) state: PlanState,
}

/// Parses both the live ACP `plan` payload and the persisted plan body — they
/// carry the same `{ source?, entries[] }` shape on purpose. Foreign JSON, so
/// every field is optional and unknown values fall back to the defaults.
pub(crate) fn parse_plan_card(payload: &serde_json::Value) -> PlanCard {
    PlanCard {
        entries: payload
            .get("entries")
            .map(|entries| serde_json::from_value(entries.clone()).unwrap_or_default())
            .unwrap_or_default(),
        source: payload
            .get("source")
            .and_then(serde_json::Value::as_str)
            .map(|raw| PlanSource::from(raw.to_string()))
            .unwrap_or_default(),
        state: PlanState::default(),
    }
}

#[cfg(test)]
mod plan_card_tests {
    use super::*;

    #[test]
    fn keeps_three_statuses_and_priority() {
        let card = parse_plan_card(&serde_json::json!({
            "entries": [
                { "content": "read", "status": "completed", "priority": "high" },
                { "content": "edit", "status": "in_progress", "priority": "medium" },
                { "content": "test", "status": "pending", "priority": "low" },
            ]
        }));
        assert_eq!(card.source, PlanSource::Acp);
        assert_eq!(card.state, PlanState::Ready);
        assert_eq!(
            card.entries,
            vec![
                PlanEntry {
                    content: "read".into(),
                    status: PlanStatus::Completed,
                    priority: PlanPriority::High,
                },
                PlanEntry {
                    content: "edit".into(),
                    status: PlanStatus::InProgress,
                    priority: PlanPriority::Medium,
                },
                PlanEntry {
                    content: "test".into(),
                    status: PlanStatus::Pending,
                    priority: PlanPriority::Low,
                },
            ]
        );
    }

    #[test]
    fn unknown_and_missing_fields_fall_back() {
        let card = parse_plan_card(&serde_json::json!({
            "source": "native",
            "entries": [{ "content": "x", "status": "blocked" }, {}],
        }));
        assert_eq!(card.source, PlanSource::Native);
        assert_eq!(card.entries[0].status, PlanStatus::Pending);
        assert_eq!(card.entries[0].priority, PlanPriority::Medium);
        assert_eq!(card.entries[1], PlanEntry::default());
    }

    #[test]
    fn junk_payloads_yield_an_empty_card() {
        assert!(parse_plan_card(&serde_json::json!({})).entries.is_empty());
        assert!(parse_plan_card(&serde_json::json!({ "entries": "nope" }))
            .entries
            .is_empty());
    }

    #[test]
    fn round_trips_through_the_persisted_shape() {
        let card = parse_plan_card(&serde_json::json!({
            "entries": [{ "content": "x", "status": "in_progress", "priority": "high" }]
        }));
        let body = serde_json::json!({ "v": 1, "source": "acp", "entries": card.entries });
        assert_eq!(parse_plan_card(&body), card);
    }
}

/// The built-in question tool. Like `propose_plan`, its result is the card's
/// body, so the tool event never renders as an ordinary tool row.
pub(crate) const ASK_USER_TOOL: &str = "ask_user";

#[derive(Serialize, Deserialize, Clone, Debug, Default, Hash, PartialEq, Eq)]
pub(crate) struct QuestionOption {
    #[serde(default)]
    pub(crate) label: String,
    #[serde(default)]
    pub(crate) description: String,
}

/// Card lifecycle. `Answered` is never persisted for the built-in source — a
/// question counts as answered once a later user message exists (the answer IS
/// that message). The ACP source persists `expired` for pendings that can no
/// longer be resolved (the bridge process died with them).
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub(crate) enum QuestionState {
    #[default]
    Pending,
    Answered,
    Expired,
}

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq)]
pub(crate) struct QuestionCard {
    pub(crate) question: String,
    pub(crate) options: Vec<QuestionOption>,
    pub(crate) allow_freeform: bool,
    pub(crate) source: PlanSource,
    /// Present only for the ACP source: the pending id `respond_ask_user` resolves.
    pub(crate) request_id: Option<String>,
    pub(crate) state: QuestionState,
}

/// Parses the `ask_user` tool body, the live ACP request payload, and the
/// reloaded row — all carry the same `{ question, options[], allow_freeform }`
/// shape; the ACP reload row adds `request_id` and `status`. Foreign JSON, so
/// every field is optional and junk degrades instead of failing the card.
pub(crate) fn parse_question_card(payload: &serde_json::Value) -> QuestionCard {
    let str_at = |key: &str| {
        payload
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    QuestionCard {
        question: str_at("question").unwrap_or_default(),
        options: payload
            .get("options")
            .map(|options| serde_json::from_value(options.clone()).unwrap_or_default())
            .unwrap_or_default(),
        allow_freeform: payload
            .get("allow_freeform")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true),
        source: str_at("source").map(PlanSource::from).unwrap_or_default(),
        request_id: str_at("request_id").filter(|id| !id.is_empty()),
        state: match str_at("status").as_deref() {
            Some("answered") => QuestionState::Answered,
            Some("expired") => QuestionState::Expired,
            _ => QuestionState::Pending,
        },
    }
}

/// Reload-time answered detection for the built-in source: a question is
/// answered once any user message follows it — the answer is that message.
/// ACP rows reload after the transcript with their own persisted status, so
/// no user message follows them and this leaves them untouched.
pub(crate) fn settle_question_cards(items: &mut [ChatItem]) {
    let last_user = items
        .iter()
        .rposition(|item| matches!(item, ChatItem::User(_)));
    let Some(last_user) = last_user else { return };
    for item in &mut items[..last_user] {
        if let ChatItem::Question(card) = item {
            if card.state == QuestionState::Pending {
                card.state = QuestionState::Answered;
            }
        }
    }
}

#[cfg(test)]
mod question_card_tests {
    use super::*;

    #[test]
    fn parses_the_tool_body() {
        let card = parse_question_card(&serde_json::json!({
            "question": "Which schema?",
            "options": [
                { "label": "v1", "description": "keep the old shape" },
                { "label": "v2" },
            ],
            "allow_freeform": false,
            "source": "native",
        }));
        assert_eq!(card.question, "Which schema?");
        assert_eq!(card.options.len(), 2);
        assert_eq!(card.options[0].label, "v1");
        assert_eq!(card.options[1].description, "");
        assert!(!card.allow_freeform);
        assert_eq!(card.source, PlanSource::Native);
        assert_eq!(card.request_id, None);
        assert_eq!(card.state, QuestionState::Pending);
    }

    #[test]
    fn parses_the_acp_reload_row() {
        let card = parse_question_card(&serde_json::json!({
            "question": "Deploy now?",
            "request_id": "ask-1",
            "status": "expired",
        }));
        assert_eq!(card.request_id.as_deref(), Some("ask-1"));
        assert_eq!(card.state, QuestionState::Expired);
        assert!(card.allow_freeform, "freeform defaults on");
        assert_eq!(card.source, PlanSource::Acp);
    }

    #[test]
    fn junk_degrades_instead_of_failing() {
        let card = parse_question_card(&serde_json::json!({ "options": "nope" }));
        assert_eq!(card.question, "");
        assert!(card.options.is_empty());
        assert_eq!(card.state, QuestionState::Pending);
    }

    #[test]
    fn settle_answers_only_questions_before_the_last_user_message() {
        let question = |state| {
            ChatItem::Question(QuestionCard {
                question: "q".into(),
                state,
                ..Default::default()
            })
        };
        let mut items = vec![
            question(QuestionState::Pending),
            ChatItem::User("the answer".into()),
            question(QuestionState::Pending),
        ];
        settle_question_cards(&mut items);
        assert!(
            matches!(&items[0], ChatItem::Question(card) if card.state == QuestionState::Answered)
        );
        assert!(
            matches!(&items[2], ChatItem::Question(card) if card.state == QuestionState::Pending),
            "a question after the last user message is still open"
        );

        let mut expired = vec![
            question(QuestionState::Expired),
            ChatItem::User("later chatter".into()),
        ];
        settle_question_cards(&mut expired);
        assert!(
            matches!(&expired[0], ChatItem::Question(card) if card.state == QuestionState::Expired),
            "settle never resurrects an expired card"
        );
    }
}

pub(crate) fn active_model_label(models: &[ModelProfile]) -> Option<String> {
    model_label(models, None)
}

pub(crate) fn model_label(models: &[ModelProfile], model_id: Option<&str>) -> Option<String> {
    // `get_session_model` marks ACP-bound frames as `acp:<label>`; show that
    // label as-is instead of falling back to the active HTTP model.
    if let Some(label) = model_id
        .and_then(|id| id.strip_prefix("acp:"))
        .filter(|label| !label.is_empty())
    {
        return Some(label.to_string());
    }
    models
        .iter()
        .find(|model| model.is_chat_model() && model_id == Some(model.id.as_str()))
        .or_else(|| {
            models
                .iter()
                .find(|model| model.active && model.is_chat_model())
        })
        .or_else(|| models.iter().find(|model| model.is_chat_model()))
        .map(|m| m.label.clone())
        .filter(|s| !s.is_empty())
}

pub(crate) fn session_model_label(
    models: &[ModelProfile],
    session_models: &HashMap<String, String>,
    session_id: Option<&str>,
) -> Option<String> {
    model_label(
        models,
        session_id.and_then(|session_id| session_models.get(session_id).map(String::as_str)),
    )
}

/// Mirrors `models::effective_context_window` in src-tauri: a configured
/// window below 4K counts as "unset" and falls back to 128K.
pub(crate) fn effective_context_window(value: u64) -> u64 {
    if value >= 4_096 {
        value
    } else {
        128_000
    }
}

/// Context window of the model a session is bound to right now, with the same
/// fallbacks as `model_label` (bound profile → active → first chat model).
/// None for ACP-bound sessions, where the agent — not an HTTP profile — owns
/// the window. The usage gauge shows this limit instead of the stale
/// `max_context` carried by the last turn's usage event, so switching models
/// or editing the profile moves the gauge immediately.
pub(crate) fn session_context_window(
    models: &[ModelProfile],
    session_models: &HashMap<String, String>,
    session_id: Option<&str>,
) -> Option<u64> {
    let bound = session_id.and_then(|session_id| session_models.get(session_id));
    if bound.is_some_and(|id| id.starts_with("acp:")) {
        return None;
    }
    models
        .iter()
        .find(|model| model.is_chat_model() && bound == Some(&model.id))
        .or_else(|| {
            models
                .iter()
                .find(|model| model.active && model.is_chat_model())
        })
        .or_else(|| models.iter().find(|model| model.is_chat_model()))
        .map(|model| effective_context_window(model.context_window))
}

#[cfg(test)]
mod model_label_tests {
    use super::model_label;

    #[test]
    fn acp_marker_shows_agent_label_instead_of_http_fallback() {
        assert_eq!(
            model_label(&[], Some("acp:Codex ACP")).as_deref(),
            Some("Codex ACP")
        );
        // A bare marker carries no label — fall through to the normal lookup.
        assert_eq!(model_label(&[], Some("acp:")), None);
    }
}

#[cfg(test)]
mod session_context_window_tests {
    use super::{session_context_window, ModelProfile};
    use std::collections::HashMap;

    fn profile(id: &str, active: bool, context_window: u64) -> ModelProfile {
        ModelProfile {
            id: id.into(),
            label: id.into(),
            provider: "openai".into(),
            api_url: String::new(),
            model: format!("model-{id}"),
            has_api_key: true,
            active,
            max_tokens: 8_192,
            context_window,
            reasoning_effort: String::new(),
            supports_vision: false,
            use_for_vision: false,
            use_for_image_generation: false,
        }
    }

    #[test]
    fn bound_session_uses_its_own_profile_window() {
        let models = vec![profile("a", true, 128_000), profile("b", false, 1_000_000)];
        let bindings = HashMap::from([("s1".to_string(), "b".to_string())]);
        assert_eq!(
            session_context_window(&models, &bindings, Some("s1")),
            Some(1_000_000)
        );
    }

    #[test]
    fn deleted_binding_falls_back_to_active_profile() {
        let models = vec![profile("a", true, 200_000)];
        let bindings = HashMap::from([("s1".to_string(), "gone".to_string())]);
        assert_eq!(
            session_context_window(&models, &bindings, Some("s1")),
            Some(200_000)
        );
    }

    #[test]
    fn window_below_4k_falls_back_to_128k() {
        let models = vec![profile("a", true, 0)];
        assert_eq!(
            session_context_window(&models, &HashMap::new(), None),
            Some(128_000)
        );
    }

    #[test]
    fn acp_bound_session_has_no_http_window() {
        let models = vec![profile("a", true, 128_000)];
        let bindings = HashMap::from([("s1".to_string(), "acp:Codex".to_string())]);
        assert_eq!(session_context_window(&models, &bindings, Some("s1")), None);
    }
}

/// Selection captured from a file preview by `api.js`'s `preview_selection`.
/// Coordinates are viewport-relative (for the fixed-position quote popup).
#[derive(Deserialize, Clone)]
pub(crate) struct PreviewSelection {
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) path: String,
    pub(crate) x: i32,
    pub(crate) y: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegionAttach {
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) jump_to_chat: bool,
}

/// Detail of the `wisp:pins-ask-ai` event: image comment pins assembled into
/// one composer message by the preview. Serialized as a struct (not
/// `serde_json::json!`) so serde-wasm-bindgen emits a plain JS object — a
/// `Value::Object` would become an ES Map the listener cannot deserialize.
#[derive(Serialize, Deserialize)]
pub(crate) struct PinsAskAi {
    pub(crate) path: String,
    pub(crate) text: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub(crate) struct ArtifactInfo {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) path: String,
    pub(crate) ts: i64,
    #[serde(default)]
    pub(crate) project_id: Option<String>,
    #[serde(default)]
    pub(crate) project_name: Option<String>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) session_title: Option<String>,
    #[serde(default)]
    pub(crate) size_bytes: Option<i64>,
    #[serde(default)]
    pub(crate) origin: Option<String>,
}

/// Immutable item in the app-global library database. Source names are
/// snapshots, so this remains useful after its project or session is deleted.
#[derive(Deserialize, Clone, PartialEq)]
pub(crate) struct LibraryItem {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) language: Option<String>,
    #[serde(default)]
    pub(crate) code: Rc<str>,
    pub(crate) content_type: Option<String>,
    pub(crate) source_project_id: String,
    pub(crate) source_project_name: String,
    pub(crate) source_session_id: String,
    pub(crate) source_session_title: String,
    pub(crate) source_path: Option<String>,
    pub(crate) created_at: i64,
}

impl LibraryItem {
    pub(crate) fn matches_code(&self, session: &str, language: &str, code: &str) -> bool {
        self.kind == "code"
            && self.source_session_id == session
            && self.language.as_deref().unwrap_or_default() == language
            && self.code.as_ref() == code
    }
}

/// Bounded Library list row. Full code/text is fetched only for the active
/// session or an opened detail.
#[derive(Deserialize, Clone, PartialEq)]
pub(crate) struct LibraryItemSummary {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) language: Option<String>,
    #[serde(default)]
    pub(crate) code_preview: String,
    pub(crate) source_project_id: String,
    pub(crate) source_project_name: String,
    pub(crate) source_session_id: String,
    pub(crate) source_session_title: String,
    pub(crate) source_path: Option<String>,
    pub(crate) created_at: i64,
}

impl LibraryItemSummary {
    pub(crate) fn matches_figure(&self, session: &str, path: &str) -> bool {
        self.kind == "figure"
            && self.source_session_id == session
            && self.source_path.as_deref().map(normalize_library_path)
                == Some(normalize_library_path(path))
    }
}

fn normalize_library_path(path: &str) -> String {
    path.strip_prefix("./")
        .or_else(|| path.strip_prefix(".\\"))
        .unwrap_or(path)
        .replace('\\', "/")
}

#[derive(Deserialize, Clone)]
pub(crate) struct LibraryItemDetail {
    #[serde(flatten)]
    pub(crate) item: LibraryItem,
    pub(crate) base64: Option<String>,
}

/// One immutable version of a library item's code — mirrors the wisp-store
/// `LibraryItemVersion` returned by `list_library_item_versions` /
/// `update_library_code`. Version 1 is the original snapshot (`id` equals the
/// item id); higher numbers are user edits.
#[derive(Deserialize, Clone, PartialEq)]
pub(crate) struct LibraryItemVersion {
    pub(crate) id: String,
    pub(crate) item_id: String,
    pub(crate) version_number: i64,
    pub(crate) parent_version_id: Option<String>,
    pub(crate) language: Option<String>,
    #[serde(default)]
    pub(crate) code: String,
    pub(crate) origin: String,
    pub(crate) created_at: i64,
}

#[derive(Deserialize, Clone, PartialEq)]
pub(crate) struct SessionSearchInfo {
    pub(crate) id: String,
    pub(crate) project_id: String,
    pub(crate) project_name: String,
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) ts: i64,
    #[serde(default)]
    pub(crate) activity_at: i64,
    #[serde(default)]
    pub(crate) status: String,
}

#[derive(Serialize, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ComposerReferenceArg {
    Artifact {
        id: String,
    },
    Session {
        id: String,
    },
    Project {
        id: String,
    },
    Skill {
        name: String,
    },
    Workflow {
        id: String,
    },
    Context {
        id: String,
    },
    Runtime {
        context_id: String,
        language: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct SshHost {
    pub(crate) alias: String,
    /// Real address (IP or domain) for manually created hosts; when absent
    /// the alias itself is the target, resolved via ~/.ssh/config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) host_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) identity_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) notes: Option<String>,
    /// `key` (default) or `password`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) auth_method: Option<String>,
    /// Whether a password is stored in the OS keyring (never the password itself).
    #[serde(default)]
    pub(crate) has_password: bool,
    /// Write-only password from the form; never returned by list APIs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) password: Option<String>,
}

/// Mirrors the `get_storage_usage` payload built in
/// src-tauri/src/settings_commands.rs — align field by field on both sides.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct StorageEntry {
    pub(crate) key: String,
    pub(crate) bytes: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct ProjectStorageUsage {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) bytes: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct StorageUsage {
    pub(crate) data_dir: String,
    #[serde(default)]
    pub(crate) projects: Vec<ProjectStorageUsage>,
    #[serde(default)]
    pub(crate) entries: Vec<StorageEntry>,
    pub(crate) total_bytes: u64,
}

/// Mirrors the token-usage payloads in crates/wisp-store/src/sessions.rs and
/// src-tauri/src/settings_commands.rs — align field by field on both sides.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct ProjectTokenUsage {
    pub(crate) project_id: String,
    pub(crate) name: String,
    pub(crate) workspace_dir: String,
    pub(crate) updated_at: i64,
    pub(crate) session_count: i64,
    pub(crate) input: i64,
    pub(crate) output: i64,
    pub(crate) reasoning: i64,
    pub(crate) cached: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct SessionTokenUsage {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) updated_at: i64,
    pub(crate) input: i64,
    pub(crate) output: i64,
    pub(crate) reasoning: i64,
    pub(crate) cached: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct SessionTokenUsagePage {
    #[serde(default)]
    pub(crate) items: Vec<SessionTokenUsage>,
    pub(crate) total: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct TokenUsageDay {
    pub(crate) date: String,
    pub(crate) tokens: i64,
    #[serde(default)]
    pub(crate) future: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct ModelTokenUsage {
    pub(crate) model: String,
    pub(crate) tokens: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct ToolCallUsage {
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) calls: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct TokenUsageOverview {
    #[serde(default)]
    pub(crate) workspaces: Vec<ProjectTokenUsage>,
    #[serde(default)]
    pub(crate) days: Vec<TokenUsageDay>,
    #[serde(default)]
    pub(crate) models: Vec<ModelTokenUsage>,
    #[serde(default)]
    pub(crate) tools: Vec<ToolCallUsage>,
}

/// Mirrors `SshTrustEdge` in src-tauri/src/run_context/transfer.rs — align
/// field by field on both sides.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct SshTrustEdge {
    pub(crate) source_context_id: String,
    pub(crate) destination_context_id: String,
    pub(crate) destination_target: String,
    #[serde(default)]
    pub(crate) destination_port: Option<u16>,
    #[serde(default)]
    pub(crate) key_path: Option<String>,
    pub(crate) managed: bool,
    pub(crate) verified_at: i64,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RevokeTrustResponse {
    pub(crate) edges: Vec<SshTrustEdge>,
    #[serde(default)]
    pub(crate) cleanup_error: Option<String>,
}

#[derive(Clone)]
pub(crate) enum ComposerAttachment {
    Uploading {
        key: String,
        name: String,
    },
    Ready {
        key: String,
        name: String,
        path: String,
    },
    Error {
        key: String,
        name: String,
        error: String,
    },
}

#[derive(Deserialize)]
pub(crate) struct UploadFileResult {
    pub(crate) ok: bool,
    pub(crate) info: Option<ArtifactInfo>,
    pub(crate) filename: Option<String>,
    pub(crate) error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct Settings {
    pub(crate) provider: String,
    pub(crate) api_url: String,
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) label: String,
    pub(crate) has_api_key: bool,
    #[serde(default)]
    pub(crate) locale: String,
    #[serde(default)]
    pub(crate) workspace_dir: String,
    #[serde(default = "default_max_iter")]
    pub(crate) max_iter: i64,
    #[serde(default = "default_auto_compact")]
    pub(crate) auto_compact: bool,
    #[serde(default = "default_follow_up_questions")]
    pub(crate) follow_up_questions: bool,
    #[serde(default = "default_resume_last_session")]
    pub(crate) resume_last_session: bool,
    #[serde(default)]
    pub(crate) max_tokens: u64,
    #[serde(default)]
    pub(crate) reasoning_effort: String,
    #[serde(default)]
    pub(crate) proxy_url: String,
    #[serde(default)]
    pub(crate) supports_vision: bool,
    #[serde(default = "default_sync_backend")]
    pub(crate) sync_backend: String,
    #[serde(default)]
    pub(crate) sync_relay_url: String,
    #[serde(default)]
    pub(crate) sync_folder: String,
    #[serde(default)]
    pub(crate) sync_relay_token: String,
    #[serde(default)]
    pub(crate) has_sync_relay_token: bool,
    #[serde(default)]
    pub(crate) pet_enabled: bool,
    #[serde(default)]
    pub(crate) pet_directory: String,
    #[serde(default = "default_notifications_enabled")]
    pub(crate) notifications_enabled: bool,
}

fn default_sync_backend() -> String {
    "relay".into()
}

fn default_notifications_enabled() -> bool {
    true
}

fn default_auto_compact() -> bool {
    true
}

fn default_follow_up_questions() -> bool {
    true
}

fn default_resume_last_session() -> bool {
    true
}

/// Mirror of `src-tauri` `channels::ChannelsStatus` (snake_case wire shape,
/// same style as `Settings`).
#[derive(Deserialize, Clone, Default)]
pub(crate) struct ChannelsStatus {
    #[serde(default)]
    pub(crate) feishu_enabled: bool,
    #[serde(default)]
    pub(crate) feishu_bound: bool,
    #[serde(default)]
    pub(crate) feishu_international: bool,
    #[serde(default)]
    pub(crate) feishu_app_id: String,
    #[serde(default)]
    pub(crate) feishu_has_secret: bool,
    #[serde(default)]
    pub(crate) feishu_state: String,
    #[serde(default)]
    pub(crate) feishu_detail: String,
    #[serde(default)]
    pub(crate) weixin_enabled: bool,
    #[serde(default)]
    pub(crate) weixin_bound: bool,
    #[serde(default)]
    pub(crate) weixin_state: String,
    #[serde(default)]
    pub(crate) weixin_detail: String,
    #[serde(default)]
    pub(crate) device: DeviceBridgeStatus,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceBridgeStatus {
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default = "default_device_bridge_mode")]
    pub(crate) mode: String,
    #[serde(default)]
    pub(crate) has_token: bool,
    #[serde(default)]
    pub(crate) state: String,
    #[serde(default)]
    pub(crate) bind_ipv4: String,
    #[serde(default = "default_device_bridge_port")]
    pub(crate) port: u16,
    #[serde(default)]
    pub(crate) url: Option<String>,
    #[serde(default)]
    pub(crate) detail: String,
}

fn default_device_bridge_port() -> u16 {
    18_766
}

fn default_device_bridge_mode() -> String {
    "lan".into()
}

impl Default for DeviceBridgeStatus {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: default_device_bridge_mode(),
            has_token: false,
            state: "stopped".into(),
            bind_ipv4: String::new(),
            port: default_device_bridge_port(),
            url: None,
            detail: String::new(),
        }
    }
}

/// Mirror of `src-tauri` `channels::WeixinBindStart`.
#[derive(Deserialize, Clone)]
pub(crate) struct WeixinBindStart {
    pub(crate) qrcode: String,
    pub(crate) qr_image: String,
}

/// Mirrors the opaque Feishu OAuth device-flow DTOs from `src-tauri`.
#[derive(Deserialize, Clone)]
pub(crate) struct FeishuBindStart {
    pub(crate) flow_id: String,
    pub(crate) qr_image: String,
    pub(crate) expires_in_seconds: u64,
}

#[derive(Deserialize, Clone)]
pub(crate) struct FeishuBindPoll {
    pub(crate) state: String,
    pub(crate) retry_after_ms: u64,
    pub(crate) app_id: String,
}

fn default_max_iter() -> i64 {
    100
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: "openai".into(),
            api_url: "https://api.deepseek.com".into(),
            model: "deepseek-v4-pro".into(),
            label: "deepseek-v4-pro".into(),
            has_api_key: false,
            locale: Locale::En.code().into(),
            workspace_dir: String::new(),
            max_iter: default_max_iter(),
            auto_compact: true,
            follow_up_questions: true,
            resume_last_session: true,
            max_tokens: 8192,
            reasoning_effort: String::new(),
            proxy_url: String::new(),
            supports_vision: false,
            sync_backend: "relay".into(),
            sync_relay_url: String::new(),
            sync_folder: String::new(),
            sync_relay_token: String::new(),
            has_sync_relay_token: false,
            pet_enabled: false,
            pet_directory: String::new(),
            notifications_enabled: true,
        }
    }
}

#[derive(Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PetStatus {
    pub(crate) enabled: bool,
    pub(crate) directory: String,
    pub(crate) asset: Option<PetAsset>,
    pub(crate) error: Option<String>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PetAsset {
    pub(crate) id: String,
    pub(crate) display_name: String,
    pub(crate) description: String,
    pub(crate) sprite_version_number: u8,
    pub(crate) spritesheet_data_url: String,
    pub(crate) frame_counts: std::collections::BTreeMap<String, u8>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectSyncResult {
    pub(crate) direction: String,
    pub(crate) uploaded_files: usize,
    pub(crate) downloaded_files: usize,
    pub(crate) skipped_paths: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct DemoInfo {
    pub(crate) id: String,
    pub(crate) title: String,
}

#[derive(Deserialize, Clone)]
pub(crate) struct Demo {
    pub(crate) title: String,
    pub(crate) request: String,
    pub(crate) response: String,
    pub(crate) thinking: Option<String>,
    #[serde(default)]
    pub(crate) items: Vec<LoadedItem>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SendMessageArgs {
    // Tauri v2 maps JS camelCase keys to snake_case params; the JS side must
    // send `sessionId` or the backend sees `None` and forks a new conversation.
    pub(crate) session_id: Option<String>,
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) attachments: Vec<String>,
    #[serde(default)]
    pub(crate) references: Vec<ComposerReferenceArg>,
    #[serde(default)]
    pub(crate) resume: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) acp_agent_id: Option<String>,
    /// Guide (#410): inject into the running turn's next loop iteration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) guide: Option<bool>,
    /// Guide (#410): roll back the interrupted turn before sending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) replace: Option<bool>,
}

/// Queue (#433): park a follow-up turn behind the running one.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnqueueTurnArgs {
    pub(crate) session_id: String,
    pub(crate) id: u64,
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) attachments: Vec<String>,
    #[serde(default)]
    pub(crate) references: Vec<ComposerReferenceArg>,
}

/// Queue (#433): edit / cancel / cut-in a parked follow-up by id.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueuedTurnActionArgs {
    pub(crate) session_id: String,
    pub(crate) id: u64,
    pub(crate) action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) message: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AcpAgentProfile {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) command: String,
    #[serde(default)]
    pub(crate) args: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcpAgentInfo {
    #[serde(default)]
    pub(crate) protocol_version: u16,
    #[serde(default)]
    pub(crate) implementation: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) capabilities: serde_json::Value,
    #[serde(default)]
    pub(crate) auth_methods: Vec<AcpAuthMethod>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AcpAuthMethod {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcpSessionUpdate {
    pub(crate) frame_id: String,
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) payload: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcpSessionState {
    pub(crate) frame_id: String,
    #[serde(default)]
    pub(crate) modes: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) config_options: Option<Vec<serde_json::Value>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcpPermissionResolved {
    pub(crate) frame_id: String,
    pub(crate) request_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub(crate) struct AcpPermissionOption {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) kind: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcpPermissionRequest {
    pub(crate) request_id: String,
    pub(crate) frame_id: String,
    #[serde(default)]
    pub(crate) tool_call: serde_json::Value,
    #[serde(default)]
    pub(crate) options: Vec<AcpPermissionOption>,
}

/// `ask-user-request`: an ACP agent's bridge `ask_user` call waiting for the
/// user. `payload` is the tool body `parse_question_card` reads.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AskUserRequest {
    pub(crate) request_id: String,
    pub(crate) frame_id: String,
    #[serde(default)]
    pub(crate) payload: serde_json::Value,
}

/// `ask-user-resolved`: the pending question was answered (or expired with the
/// turn) and its card should settle.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AskUserResolved {
    pub(crate) request_id: String,
    pub(crate) frame_id: String,
    #[serde(default)]
    pub(crate) expired: bool,
}

#[derive(Deserialize, Clone)]
pub(crate) struct SessionInfo {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) ts: i64,
    #[serde(default)]
    pub(crate) folder_id: Option<String>,
    /// Source session this one was branched from; nested under it in the sidebar.
    #[serde(default)]
    pub(crate) branched_from: Option<String>,
    #[serde(default)]
    pub(crate) pinned: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionBranchDeltaMessage {
    pub(crate) seq: i64,
    pub(crate) role: String,
    pub(crate) text: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionBranchCandidate {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) is_main: bool,
    pub(crate) new_message_count: usize,
    pub(crate) messages: Vec<SessionBranchDeltaMessage>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionBranchComparison {
    pub(crate) main_session_id: String,
    pub(crate) common_ancestor_messages: usize,
    pub(crate) guard_hash: String,
    pub(crate) candidates: Vec<SessionBranchCandidate>,
    #[serde(default)]
    pub(crate) analysis: Option<String>,
    #[serde(default)]
    pub(crate) analysis_error: Option<String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionBranchConvergence {
    pub(crate) main_session_id: String,
    pub(crate) selected_session_id: String,
    pub(crate) removed_session_ids: Vec<String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct Exploration {
    pub(crate) id: String,
    pub(crate) checkpoint_id: String,
    pub(crate) frame_id: String,
    pub(crate) name: String,
    pub(crate) status: String,
    pub(crate) workspace_dir: String,
    pub(crate) workspace_backend: String,
    pub(crate) scope_generation: i64,
    pub(crate) warnings_json: String,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) promoted_at: Option<i64>,
    pub(crate) archived_at: Option<i64>,
    pub(crate) discarded_at: Option<i64>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExplorationSummary {
    pub(crate) exploration: Exploration,
    pub(crate) source_frame_id: String,
    pub(crate) isolation_summary_json: String,
}

impl ExplorationSummary {
    pub(crate) fn isolation_is_full(&self) -> bool {
        serde_json::from_str::<serde_json::Value>(&self.isolation_summary_json)
            .ok()
            .and_then(|value| value.get("partial").and_then(serde_json::Value::as_bool))
            != Some(true)
            && serde_json::from_str::<Vec<serde_json::Value>>(
                &self.exploration.warnings_json,
            )
            .map_or(true, |warnings| warnings.is_empty())
    }
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExplorationFileDelta {
    pub(crate) path: String,
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) before: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) after: Option<serde_json::Value>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExplorationArtifactDelta {
    pub(crate) logical_key: String,
    pub(crate) before_artifact_id: Option<String>,
    pub(crate) before_version_id: Option<String>,
    pub(crate) after_artifact_id: String,
    pub(crate) after_version_id: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExplorationExternalResource {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) uri: String,
    pub(crate) version: Option<String>,
    pub(crate) checksum: Option<String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExplorationEffect {
    pub(crate) id: String,
    pub(crate) effect_kind: String,
    pub(crate) recoverability: String,
    pub(crate) target_summary: String,
    pub(crate) metadata_json: String,
    pub(crate) created_at: i64,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExplorationDiff {
    pub(crate) exploration_id: String,
    pub(crate) files: Vec<ExplorationFileDelta>,
    pub(crate) artifacts: Vec<ExplorationArtifactDelta>,
    pub(crate) runs: Vec<RunRecord>,
    pub(crate) decisions: Vec<ResearchNode>,
    pub(crate) research_edges: Vec<ResearchEdge>,
    pub(crate) external_resources: Vec<ExplorationExternalResource>,
    pub(crate) external_effects: Vec<ExplorationEffect>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExplorationMainlineChanges {
    pub(crate) files: Vec<ExplorationFileDelta>,
    pub(crate) artifact_keys: Vec<String>,
    pub(crate) entity_keys: Vec<String>,
    pub(crate) source_message_head: i64,
    pub(crate) source_ui_event_head: i64,
    pub(crate) state_generation: i64,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromotionBlocker {
    pub(crate) code: String,
    pub(crate) message: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromotionEligibility {
    pub(crate) eligible: bool,
    pub(crate) code: Option<String>,
    pub(crate) reasons: Vec<PromotionBlocker>,
    pub(crate) expected_guard_hash: String,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExplorationPromotionPreview {
    pub(crate) exploration: Exploration,
    pub(crate) diff: ExplorationDiff,
    pub(crate) mainline_changes: ExplorationMainlineChanges,
    pub(crate) eligibility: PromotionEligibility,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExplorationPromotionResult {
    pub(crate) exploration: Exploration,
    pub(crate) promotion_id: String,
    pub(crate) adopted_frame_id: String,
}

/// One Codex CLI or Claude Code conversation offered by the import modal.
/// `state` is "new" | "imported" | "updatable".
#[derive(Deserialize, Clone, PartialEq)]
pub(crate) struct ExternalSessionInfo {
    pub(crate) path: String,
    pub(crate) session_id: String,
    pub(crate) title: String,
    pub(crate) cwd: String,
    pub(crate) message_count: usize,
    pub(crate) last_active_at: i64,
    pub(crate) state: String,
}

#[derive(Deserialize, Clone, PartialEq)]
pub(crate) struct ExternalSessionPreviewLine {
    pub(crate) role: String,
    pub(crate) text: String,
}

#[derive(Deserialize, Clone, Default)]
pub(crate) struct ExternalImportSummary {
    pub(crate) imported: usize,
    pub(crate) updated: usize,
    pub(crate) skipped: usize,
    pub(crate) failed: usize,
    #[serde(default)]
    pub(crate) synced_paths: Vec<String>,
}

#[derive(Deserialize, Serialize, Clone)]
pub(crate) struct SessionCursor {
    pub(crate) ts: i64,
    pub(crate) id: String,
}

#[derive(Deserialize)]
pub(crate) struct SessionPage {
    pub(crate) items: Vec<SessionInfo>,
    pub(crate) next_cursor: Option<SessionCursor>,
    pub(crate) running_ids: Vec<String>,
}

#[derive(Deserialize, Clone)]
pub(crate) struct FolderInfo {
    pub(crate) id: String,
    pub(crate) name: String,
}

/// A transcript row returned by `load_session`.
#[derive(Deserialize, Clone)]
pub(crate) struct LoadedItem {
    pub(crate) role: String,
    pub(crate) text: String,
    pub(crate) tool_name: Option<String>,
    pub(crate) ok: Option<bool>,
    #[serde(default)]
    pub(crate) duration_ms: Option<u64>,
    #[serde(default)]
    pub(crate) input: String,
    #[serde(default)]
    pub(crate) model_name: Option<String>,
    #[serde(default)]
    pub(crate) call_id: Option<String>,
    #[serde(default)]
    pub(crate) kind: Option<String>,
    #[serde(default)]
    pub(crate) status: Option<String>,
    #[serde(default)]
    pub(crate) locations: Option<String>,
    #[serde(default)]
    pub(crate) resources: Vec<MessageResource>,
    #[serde(default)]
    pub(crate) details: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub(crate) struct LoadedSessionPage {
    pub(crate) items: Vec<LoadedItem>,
    pub(crate) next_before_seq: Option<i64>,
    pub(crate) user_offset: usize,
    #[serde(default)]
    pub(crate) outline: Vec<SessionOutlineItem>,
    #[serde(default)]
    pub(crate) presentations: Vec<LoadedPresentation>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionOutlineItem {
    pub(crate) user_index: usize,
    #[serde(default)]
    pub(crate) seq: Option<i64>,
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) sent_at: Option<i64>,
    #[serde(default)]
    pub(crate) response_at: Option<i64>,
}

#[derive(Deserialize, Clone)]
pub(crate) struct LoadedPresentation {
    #[serde(default)]
    pub(crate) presentation_id: String,
    pub(crate) presentation_kind: String,
    pub(crate) payload: serde_json::Value,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct TranscriptPageState {
    pub(crate) next_before_seq: Option<i64>,
    pub(crate) user_offset: usize,
    pub(crate) loading: bool,
    pub(crate) window_user_start: usize,
}

impl LoadedItem {
    pub(crate) fn into_chat(self) -> ChatItem {
        match self.role.as_str() {
            "user" => ChatItem::User(self.text),
            "reasoning" => ChatItem::Reasoning(self.text),
            "review" => serde_json::from_str(&self.text)
                .map(ChatItem::Review)
                .unwrap_or_else(|_| ChatItem::Assistant {
                    text: self.text,
                    model: None,
                    resources: self.resources,
                }),
            "plan" => serde_json::from_str(&self.text)
                .map(|payload: serde_json::Value| ChatItem::Plan(parse_plan_card(&payload)))
                .unwrap_or_else(|_| ChatItem::Assistant {
                    text: self.text,
                    model: None,
                    resources: self.resources,
                }),
            "question" => serde_json::from_str(&self.text)
                .map(|payload: serde_json::Value| ChatItem::Question(parse_question_card(&payload)))
                .unwrap_or_else(|_| ChatItem::Assistant {
                    text: self.text,
                    model: None,
                    resources: self.resources,
                }),
            "acp_tool" => ChatItem::AcpTool {
                call_id: self.call_id.unwrap_or_default(),
                title: self.tool_name.unwrap_or_else(|| "ACP tool".into()),
                kind: self.kind.unwrap_or_default(),
                status: self.status.unwrap_or_else(|| "completed".into()),
                content: self.text,
                locations: self.locations.unwrap_or_default(),
            },
            "tool" => ChatItem::Tool {
                name: self.tool_name.unwrap_or_else(|| "tool".into()),
                ok: self.ok,
                input: self.input,
                output: self.text,
                started_at_ms: None,
                duration_ms: self.duration_ms,
                call_key: None,
                details: self.details,
            },
            "file_changed" => ChatItem::FileChanged(self.text),
            "usage" => {
                let v: serde_json::Value = serde_json::from_str(&self.text).unwrap_or_default();
                let n = |k: &str| v.get(k).and_then(serde_json::Value::as_u64).unwrap_or(0);
                ChatItem::Usage {
                    input: n("input"),
                    output: n("output"),
                    reasoning: n("reasoning"),
                    cached: n("cached"),
                    ctx_tokens: n("ctx_tokens") as usize,
                    max_context: n("max_context") as usize,
                    context_usage: v
                        .get("context_usage")
                        .cloned()
                        .and_then(|value| serde_json::from_value(value).ok())
                        .unwrap_or_default(),
                }
            }
            "compaction" => {
                let value: serde_json::Value = serde_json::from_str(&self.text).unwrap_or_default();
                ChatItem::Compaction {
                    before: value
                        .get("before")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or_default() as usize,
                    after: value
                        .get("after")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or_default() as usize,
                    strategy: value
                        .get("strategy")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("manual")
                        .to_string(),
                }
            }
            _ => ChatItem::Assistant {
                text: self.text,
                model: self.model_name,
                resources: self.resources,
            },
        }
    }
}

#[derive(Clone, PartialEq)]
pub(crate) struct TableData {
    pub(crate) headers: Vec<String>,
    pub(crate) rows: Vec<Vec<String>>,
}

#[derive(Clone, PartialEq)]
pub(crate) enum PreviewData {
    Table(Rc<TableData>),
    Latex { tex: String, display: bool },
    File { path: String, kind: String },
    Fasta(Rc<str>),
}

#[derive(Clone, PartialEq)]
pub(crate) struct Artifact {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) kind: &'static str,
    pub(crate) data: PreviewData,
    /// Transcript item that most recently produced or mentioned this artifact.
    pub(crate) source_item: usize,
    pub(crate) superseded: bool,
}

#[derive(Deserialize)]
pub(crate) struct FileContent {
    pub(crate) path: String,
    pub(crate) mime: String,
    pub(crate) text: Option<String>,
    pub(crate) base64: Option<String>,
    /// Set when the backend returned only a leading prefix of a large text file.
    #[serde(default)]
    pub(crate) truncated: bool,
    /// Full on-disk size (bytes), present when known.
    #[serde(default)]
    pub(crate) total_bytes: Option<u64>,
}

#[derive(Deserialize, Clone)]
pub(crate) struct DirEntry {
    pub(crate) name: String,
    pub(crate) is_dir: bool,
    pub(crate) size: u64,
}

#[derive(Deserialize, Clone)]
pub(crate) struct DirectoryListing {
    pub(crate) path: String,
    pub(crate) entries: Vec<DirEntry>,
}

#[derive(Deserialize, Clone)]
pub(crate) struct FileSearchHit {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) is_dir: bool,
    pub(crate) size: u64,
}

#[derive(Deserialize, Clone)]
pub(crate) struct ScratchChatInfo {
    #[serde(rename = "sessionId")]
    pub(crate) session_id: String,
}

#[derive(Deserialize, Clone)]
pub(crate) struct ProjectInfo {
    #[serde(default)]
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) root: String,
    pub(crate) skill_count: usize,
    pub(crate) mcp_server_count: usize,
    pub(crate) memory_file_count: usize,
}

#[derive(Clone, Deserialize, PartialEq)]
pub(crate) struct ProjectSummary {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) workspace_dir: String,
    #[serde(default)]
    pub(crate) session_count: i64,
    #[serde(default)]
    pub(crate) artifact_count: i64,
    #[serde(default)]
    pub(crate) updated_at: i64,
    #[serde(default)]
    pub(crate) running_count: i64,
    #[serde(default)]
    pub(crate) needs_you_count: i64,
    #[serde(default)]
    pub(crate) sync_configured: bool,
    #[serde(default)]
    pub(crate) last_synced_at: Option<i64>,
}

/// Editable project settings (Project Settings modal). `agent_context` is the
/// project's `.wisp/WISP.md`, injected into every seeded system prompt.
#[derive(Clone, Deserialize, Default)]
pub(crate) struct ProjectSettings {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) agent_context: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionStatusKind {
    Running,
    NeedsYou,
    Complete,
}

impl SessionStatusKind {
    pub(crate) fn from_str(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "needs_you" => Self::NeedsYou,
            _ => Self::Complete,
        }
    }

    pub(crate) fn i18n_key(self) -> &'static str {
        match self {
            Self::Running => "sess_status.running",
            Self::NeedsYou => "sess_status.needs_you",
            Self::Complete => "sess_status.complete",
        }
    }

    pub(crate) fn css(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::NeedsYou => "needs-you",
            Self::Complete => "complete",
        }
    }
}

/// One configured model profile (mirrors `models::ModelProfile` in src-tauri).
#[derive(Clone, Deserialize)]
pub(crate) struct ModelProfile {
    pub(crate) id: String,
    pub(crate) label: String,
    #[serde(default)]
    pub(crate) provider: String,
    #[serde(default)]
    pub(crate) api_url: String,
    #[serde(default)]
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) has_api_key: bool,
    #[serde(default)]
    pub(crate) active: bool,
    #[serde(default)]
    pub(crate) max_tokens: u64,
    #[serde(default = "default_model_context_window")]
    pub(crate) context_window: u64,
    #[serde(default)]
    pub(crate) reasoning_effort: String,
    #[serde(default)]
    pub(crate) supports_vision: bool,
    #[serde(default)]
    pub(crate) use_for_vision: bool,
    #[serde(default)]
    pub(crate) use_for_image_generation: bool,
}

impl ModelProfile {
    pub(crate) fn is_chat_model(&self) -> bool {
        !self.model.trim().eq_ignore_ascii_case("gpt-image-2")
    }
}

/// A user-definable agent persona (mirrors `specialists::Specialist` in src-tauri).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct Specialist {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) icon: String,
    #[serde(default)]
    pub(crate) color: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) instructions: String,
    #[serde(default)]
    pub(crate) model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) review_backend: Option<ReviewBackendConfig>,
    #[serde(default)]
    pub(crate) skills: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) connectors: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) builtin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ReviewBackendConfig {
    FollowSession,
    HttpModel {
        #[serde(default)]
        profile_id: String,
    },
    AcpAgent {
        profile_id: String,
    },
}

impl ReviewBackendConfig {
    pub(crate) fn follow_session() -> Self {
        Self::FollowSession
    }

    pub(crate) fn http(profile_id: impl Into<String>) -> Self {
        Self::HttpModel {
            profile_id: profile_id.into(),
        }
    }

    pub(crate) fn acp(profile_id: impl Into<String>) -> Self {
        Self::AcpAgent {
            profile_id: profile_id.into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewerBackendTestResult {
    pub(crate) backend: String,
    pub(crate) model: String,
    pub(crate) status: String,
    pub(crate) summary: String,
}

#[derive(Clone, Deserialize)]
pub(crate) struct RecentSession {
    pub(crate) id: String,
    pub(crate) project_id: String,
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) ts: i64,
    #[serde(default)]
    pub(crate) status: String,
}

#[derive(Clone, serde::Deserialize, PartialEq)]
pub(crate) struct SkillRow {
    pub(crate) name: String,
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    #[serde(default)]
    pub(crate) scope: String,
    pub(crate) enabled: bool,
    pub(crate) builtin: bool,
    #[serde(default)]
    pub(crate) managed: bool,
    #[serde(default)]
    pub(crate) managed_by: Option<String>,
    pub(crate) dir: String,
}

#[derive(Clone, serde::Deserialize, PartialEq)]
pub(crate) struct PluginRow {
    pub(crate) id: String,
    pub(crate) version: String,
    pub(crate) display_name: String,
    pub(crate) description: String,
    pub(crate) author: String,
    pub(crate) license: String,
    pub(crate) source_uri: String,
    pub(crate) archive_sha256: String,
    pub(crate) trust_state: String,
    pub(crate) enabled: bool,
    pub(crate) skill_count: usize,
    #[serde(default)]
    pub(crate) skill_names: Vec<String>,
    pub(crate) mcp_server_count: usize,
    #[serde(default)]
    pub(crate) commands: Vec<String>,
    #[serde(default)]
    pub(crate) runtime_status: String,
    #[serde(default)]
    pub(crate) runtime_errors: Vec<String>,
}

#[derive(Clone, serde::Deserialize)]
pub(crate) struct ConnRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) enabled: bool,
    pub(crate) transport: ConnTransport,
}
#[derive(Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum ConnTransport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: Vec<(String, String)>,
        #[serde(default)]
        auth: String,
    },
}
#[derive(Clone, serde::Deserialize)]
pub(crate) struct ConnView {
    pub(crate) connections: Vec<ConnRow>,
}

// Multi-level connectors tree (bundled bio-tools domains + custom connections).
fn default_tool_mode() -> String {
    "allow".into()
}
#[derive(Clone, serde::Deserialize)]
pub(crate) struct ConnectorTool {
    pub(crate) name: String,
    #[serde(default = "default_tool_mode")]
    pub(crate) mode: String,
    #[serde(default)]
    pub(crate) description: String,
}
#[derive(Clone, serde::Deserialize)]
pub(crate) struct ConnectorInfo {
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) enabled: bool,
    pub(crate) skip_approvals: bool,
    pub(crate) transport: String,
    pub(crate) subtitle: String,
    #[serde(default)]
    pub(crate) auth: String,
    pub(crate) tools: Vec<ConnectorTool>,
}
#[derive(Clone, serde::Deserialize)]
pub(crate) struct ConnectorsView {
    pub(crate) connectors: Vec<ConnectorInfo>,
    /// Global approval scope: "full" | "auto" | "ask".
    pub(crate) scope: String,
}

#[derive(Clone, serde::Deserialize)]
pub(crate) struct ApprovalGrantRow {
    pub(crate) scope: String,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) project_id: Option<String>,
    pub(crate) kind: String,
    pub(crate) target: String,
    pub(crate) label: String,
}

// Simple flat form state (kind + raw text fields; args/env/headers entered as text, parsed on save).
#[derive(Clone, Default)]
pub(crate) struct ConnForm {
    pub(crate) id: Option<String>,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) command: String,
    pub(crate) args: String,
    pub(crate) url: String,
    pub(crate) headers: String,
    pub(crate) auth: String,
    pub(crate) enabled: bool,
}

#[derive(Clone, Default)]
pub(crate) struct ModelForm {
    pub(crate) id: Option<String>,
    pub(crate) label: String,
    pub(crate) provider: String,
    pub(crate) api_url: String,
    pub(crate) model: String,
    pub(crate) max_tokens: u64,
    pub(crate) context_window: u64,
    pub(crate) reasoning_effort: String,
    pub(crate) supports_vision: bool,
    pub(crate) use_for_vision: bool,
    pub(crate) use_for_image_generation: bool,
}

fn default_model_context_window() -> u64 {
    128_000
}

#[derive(Deserialize, Clone)]
pub(crate) struct MemoryFile {
    pub(crate) name: String,
    pub(crate) bytes: u64,
}

#[derive(Deserialize, Clone)]
pub(crate) struct MemoryView {
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) project_id: String,
    #[serde(default)]
    pub(crate) project_name: String,
    pub(crate) today_file: String,
    pub(crate) files: Vec<MemoryFile>,
    #[serde(default)]
    pub(crate) global_memories: Vec<GlobalMemory>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct GlobalMemory {
    pub(crate) id: String,
    pub(crate) content: String,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AutoFailureAnalysisSettings {
    pub(crate) enabled: bool,
    pub(crate) failure_rate_threshold: u8,
    pub(crate) minimum_failures: u16,
}

impl Default for AutoFailureAnalysisSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            failure_rate_threshold: 30,
            minimum_failures: 2,
        }
    }
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct TurnMemoryProposal {
    pub(crate) session_id: String,
    pub(crate) turn_index: usize,
    pub(crate) scope: String,
    pub(crate) content: String,
    pub(crate) trigger: String,
    pub(crate) tool_calls: usize,
    pub(crate) failed_tool_calls: usize,
    pub(crate) failure_rate: f64,
    #[serde(default)]
    pub(crate) global_memories: Vec<GlobalMemory>,
}

#[derive(Deserialize, Clone)]
pub(crate) struct BootstrapStatus {
    pub(crate) skills_loaded: usize,
    pub(crate) python_ok: bool,
    #[serde(default)]
    pub(crate) python_initializing: bool,
    pub(crate) mcp_catalog: usize,
    pub(crate) uv_ok: bool,
    pub(crate) node_ok: bool,
    pub(crate) sci_ok: bool,
    pub(crate) pixi_ok: bool,
    pub(crate) app_version: String,
    #[serde(default)]
    pub(crate) os: String,
    #[serde(default)]
    pub(crate) arch: String,
    pub(crate) workspace: String,
    /// Launch timings (`total=…ms … window_ready=…ms`) for bug reports.
    #[serde(default)]
    pub(crate) startup: String,
    pub(crate) errors: Vec<String>,
}

#[derive(Deserialize, Clone)]
pub(crate) struct UpdateCheck {
    pub(crate) current_version: String,
    pub(crate) latest_version: String,
    pub(crate) update_available: bool,
    pub(crate) release_url: String,
    #[serde(default)]
    pub(crate) notes: String,
    #[serde(default)]
    pub(crate) install_supported: bool,
    #[serde(default)]
    pub(crate) downloaded: bool,
    #[serde(default)]
    pub(crate) downloading: bool,
}

#[derive(Deserialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub(crate) enum UpdateDownloadEvent {
    Started { content_length: Option<u64> },
    Progress { chunk_length: u64 },
    Verified,
}

#[derive(Deserialize, Clone)]
pub(crate) struct Capabilities {
    pub(crate) memory_files: Vec<MemoryFile>,
    #[serde(default)]
    pub(crate) skill_counts: CapabilitySourceCounts,
    #[serde(default)]
    pub(crate) mcp_counts: CapabilitySourceCounts,
}

#[derive(Deserialize, Clone, Copy, Default)]
pub(crate) struct CapabilitySourceCounts {
    pub(crate) bundled: usize,
    pub(crate) project: usize,
}

#[derive(Deserialize, Clone)]
pub(crate) struct OnboardingState {
    pub(crate) show: bool,
}

/// Mirrors `wisp_store::ResearchNode`. `kind` stays a plain string because the
/// backend enum serializes to snake_case and the pane only ever groups on it.
/// `metadata_json` arrives as the store's raw JSON string, not an object.
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResearchNode {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) ref_id: Option<String>,
    pub(crate) metadata_json: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResearchEdge {
    pub(crate) source_id: String,
    pub(crate) target_id: String,
    pub(crate) relation: String,
    pub(crate) metadata_json: String,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ResearchGraph {
    #[serde(default)]
    pub(crate) nodes: Vec<ResearchNode>,
    #[serde(default)]
    pub(crate) edges: Vec<ResearchEdge>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicationInfo {
    pub(crate) id: String,
    pub(crate) project_id: String,
    pub(crate) title: String,
    pub(crate) description: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicationRevisionInfo {
    pub(crate) id: String,
    pub(crate) publication_id: String,
    pub(crate) parent_revision_id: Option<String>,
    pub(crate) revision_number: i64,
    pub(crate) label: String,
    pub(crate) state: String,
    pub(crate) capability_level: String,
    pub(crate) manifest_sha256: Option<String>,
    pub(crate) frozen_at: Option<i64>,
    pub(crate) published_at: Option<i64>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicationItemInfo {
    pub(crate) id: String,
    pub(crate) revision_id: String,
    pub(crate) parent_item_id: Option<String>,
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) content: String,
    pub(crate) ordinal: i64,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicationItemLinkInfo {
    pub(crate) source_item_id: String,
    pub(crate) target_item_id: String,
    pub(crate) relation: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicationEvidenceBinding {
    pub(crate) id: String,
    pub(crate) revision_id: String,
    pub(crate) item_id: Option<String>,
    pub(crate) source_kind: String,
    pub(crate) source_id: String,
    pub(crate) purpose: String,
    pub(crate) supported_claim_item_id: Option<String>,
    pub(crate) selection_state: String,
    pub(crate) review_state: String,
    pub(crate) reproduction_state: String,
    pub(crate) visibility: String,
    pub(crate) source_snapshot_json: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct PublicationEvidenceReview {
    pub(crate) binding_id: String,
    pub(crate) reviewer: String,
    pub(crate) method: String,
    pub(crate) verified_at: i64,
    pub(crate) result: String,
    pub(crate) report_json: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicationEvidenceSupersession {
    pub(crate) old_binding_id: String,
    pub(crate) new_binding_id: String,
    pub(crate) reason: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicationWaiverInfo {
    pub(crate) finding_code: String,
    pub(crate) author: String,
    pub(crate) reason: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct PublicationReadinessFinding {
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) binding_id: Option<String>,
    pub(crate) source_id: Option<String>,
    pub(crate) waivable: bool,
    pub(crate) waived: bool,
    pub(crate) waiver: Option<PublicationWaiverInfo>,
    pub(crate) details: serde_json::Value,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct PublicationReadinessInfo {
    pub(crate) revision_id: String,
    pub(crate) target_visibility: String,
    pub(crate) capability_level: String,
    #[serde(default)]
    pub(crate) blockers: Vec<PublicationReadinessFinding>,
    #[serde(default)]
    pub(crate) warnings: Vec<PublicationReadinessFinding>,
    #[serde(default)]
    pub(crate) omissions: Vec<PublicationReadinessFinding>,
    pub(crate) manifest_sha256: String,
    pub(crate) can_freeze: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicationEvidenceDriftInfo {
    pub(crate) binding_id: String,
    pub(crate) bound_version_id: String,
    pub(crate) bound_version_number: i64,
    pub(crate) latest_version_id: String,
    pub(crate) latest_version_number: i64,
    pub(crate) has_drift: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicationLineageInfo {
    pub(crate) binding_id: String,
    pub(crate) source_label: String,
    pub(crate) quality: String,
    #[serde(default)]
    pub(crate) bases: Vec<String>,
    pub(crate) exact_version_id: Option<String>,
    pub(crate) version_number: Option<i64>,
    pub(crate) checksum: Option<String>,
    pub(crate) capture_timing: Option<String>,
    pub(crate) producing_run_id: Option<String>,
    pub(crate) run_input_count: usize,
    pub(crate) run_output_count: usize,
    pub(crate) code_snapshot_count: usize,
    pub(crate) environment_captured: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct CapsuleBuildInfo {
    pub(crate) id: String,
    pub(crate) revision_id: String,
    pub(crate) format: String,
    pub(crate) visibility: String,
    pub(crate) status: String,
    pub(crate) output_path: Option<String>,
    pub(crate) revision_manifest_sha256: String,
    pub(crate) archive_sha256: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) created_at: i64,
    pub(crate) completed_at: Option<i64>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReproductionRunInfo {
    pub(crate) id: String,
    pub(crate) source_run_id: String,
    pub(crate) status: String,
    pub(crate) capability_level: String,
    pub(crate) expected_environment_hash: Option<String>,
    pub(crate) actual_environment_hash: String,
    pub(crate) environment_matched: bool,
    pub(crate) stdout_tail: Option<String>,
    pub(crate) stderr_tail: Option<String>,
    pub(crate) exit_code: Option<i64>,
    pub(crate) error: Option<String>,
    pub(crate) created_at: i64,
    pub(crate) completed_at: Option<i64>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReproductionResultInfo {
    pub(crate) reproduction_run_id: String,
    pub(crate) output_id: String,
    pub(crate) output_path: String,
    pub(crate) comparator_kind: String,
    pub(crate) required: bool,
    pub(crate) passed: bool,
    pub(crate) report_json: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct PublicationWorkspaceInfo {
    #[serde(default)]
    pub(crate) publications: Vec<PublicationInfo>,
    pub(crate) publication: Option<PublicationInfo>,
    #[serde(default)]
    pub(crate) revisions: Vec<PublicationRevisionInfo>,
    pub(crate) revision: Option<PublicationRevisionInfo>,
    #[serde(default)]
    pub(crate) items: Vec<PublicationItemInfo>,
    #[serde(default)]
    pub(crate) item_links: Vec<PublicationItemLinkInfo>,
    #[serde(default)]
    pub(crate) bindings: Vec<PublicationEvidenceBinding>,
    #[serde(default)]
    pub(crate) reviews: Vec<PublicationEvidenceReview>,
    #[serde(default)]
    pub(crate) supersessions: Vec<PublicationEvidenceSupersession>,
    #[serde(default)]
    pub(crate) waivers: Vec<PublicationWaiverInfo>,
    pub(crate) readiness: Option<PublicationReadinessInfo>,
    #[serde(default)]
    pub(crate) drift: Vec<PublicationEvidenceDriftInfo>,
    #[serde(default)]
    pub(crate) lineage: Vec<PublicationLineageInfo>,
    #[serde(default)]
    pub(crate) capsule_builds: Vec<CapsuleBuildInfo>,
    pub(crate) effective_capability_level: Option<String>,
    #[serde(default)]
    pub(crate) reproduction_runs: Vec<ReproductionRunInfo>,
    #[serde(default)]
    pub(crate) reproduction_results: Vec<ReproductionResultInfo>,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct PublicationFreezeOutcome {
    pub(crate) frozen: bool,
    pub(crate) revision: PublicationRevisionInfo,
    pub(crate) readiness: PublicationReadinessInfo,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RightTab {
    Artifacts,
    Agents,
    Notebook,
    Highlights,
    File,
    Provenance,
    Hosts,
    SideChat,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct QuickAction {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) icon: String,
    pub(crate) context: String,
    pub(crate) workflow_template_id: String,
    pub(crate) enabled: bool,
    pub(crate) sort_order: i64,
    pub(crate) builtin: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct WorkflowTemplate {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) proposal: DynamicAgentWorkflowProposal,
    pub(crate) builtin: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct QuickActionRun {
    pub(crate) action: QuickAction,
    pub(crate) session_id: String,
    pub(crate) display_message: String,
    pub(crate) workflow: AgentWorkflowSnapshot,
    pub(crate) started: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentWorkflowSnapshot {
    pub(crate) workflow: AgentWorkflow,
    pub(crate) delegation_enabled: bool,
    #[serde(default)]
    pub(crate) approval_policy: AgentApprovalPolicy,
    pub(crate) dynamic: DynamicAgentWorkflowSummary,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentCompletionPolicy {
    #[default]
    Inline,
    Background,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AgentCompletionSettings {
    #[serde(default)]
    pub(crate) policy: AgentCompletionPolicy,
    #[serde(default)]
    pub(crate) auto_resume: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentApprovalPolicy {
    ReviewAll,
    AutoSafe,
}

impl Default for AgentApprovalPolicy {
    fn default() -> Self {
        Self::ReviewAll
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentExecutorSelection {
    pub(crate) kind: String,
    pub(crate) profile_id: Option<String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentCapabilityOption {
    pub(crate) id: String,
    pub(crate) display_name: String,
    pub(crate) description: String,
    pub(crate) risk: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentSkillOption {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) scope: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentModelOption {
    pub(crate) id: String,
    pub(crate) external: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExecutorProfileSummary {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) profile_id: Option<String>,
    pub(crate) display_name: String,
    pub(crate) available: bool,
    pub(crate) supported_features: Vec<String>,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DynamicAgentEditorOptions {
    pub(crate) capabilities: Vec<AgentCapabilityOption>,
    #[serde(default)]
    pub(crate) skills: Vec<AgentSkillOption>,
    pub(crate) models: Vec<AgentModelOption>,
    pub(crate) executors: Vec<ExecutorProfileSummary>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct AgentBudgetProposal {
    pub(crate) max_tokens: Option<u32>,
    pub(crate) max_tool_calls: Option<u32>,
    pub(crate) max_cost_microunits: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkflowTaskKind {
    #[default]
    Agent,
    RunActivity,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct RunActivityProposal {
    pub(crate) activity: String,
    pub(crate) context_id: String,
    pub(crate) input_task_id: String,
    pub(crate) spec_output_pointer: String,
    pub(crate) max_candidates: u32,
    pub(crate) max_wall_seconds: u64,
    pub(crate) max_evaluator_seconds: u64,
    pub(crate) max_cost_microunits: u64,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct RunActivitySpec {
    pub(crate) activity: String,
    pub(crate) context_id: String,
    pub(crate) context_revision: String,
    pub(crate) input_task_id: String,
    pub(crate) spec_output_pointer: String,
    pub(crate) max_candidates: u32,
    pub(crate) max_wall_seconds: u64,
    pub(crate) max_evaluator_seconds: u64,
    pub(crate) max_cost_microunits: u64,
    pub(crate) provider_profile_id: Option<String>,
    pub(crate) model_profile_id: Option<String>,
    pub(crate) approval_reasons: Vec<String>,
    pub(crate) integrity_hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct DynamicAgentTaskProposal {
    pub(crate) id: String,
    pub(crate) instruction: String,
    pub(crate) depends_on: Vec<String>,
    #[serde(default)]
    pub(crate) task_kind: WorkflowTaskKind,
    #[serde(default)]
    pub(crate) run_activity: Option<RunActivityProposal>,
    #[serde(default)]
    pub(crate) capabilities: Vec<String>,
    #[serde(default)]
    pub(crate) skill_ids: Vec<String>,
    pub(crate) specialist_id: Option<String>,
    pub(crate) output_schema: Option<serde_json::Value>,
    pub(crate) isolated: bool,
    pub(crate) model_id: Option<String>,
    pub(crate) executor: Option<AgentExecutorSelection>,
    pub(crate) budget: Option<AgentBudgetProposal>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct DynamicAgentWorkflowProposal {
    pub(crate) goal: String,
    pub(crate) context: String,
    pub(crate) approval_policy: AgentApprovalPolicy,
    pub(crate) tasks: Vec<DynamicAgentTaskProposal>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct SkillPortfolioTaskSummary {
    pub(crate) id: String,
    pub(crate) rationale: String,
    pub(crate) skill_ids: Vec<String>,
    pub(crate) depends_on: Vec<String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct PortfolioPlanSummary {
    pub(crate) planner_model_id: String,
    pub(crate) planner_model_label: String,
    pub(crate) rationale: String,
    pub(crate) tasks: Vec<SkillPortfolioTaskSummary>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct SkillPortfolioDraft {
    pub(crate) plan: PortfolioPlanSummary,
    pub(crate) proposal: DynamicAgentWorkflowProposal,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentExecutorSummary {
    pub(crate) kind: String,
    pub(crate) profile_id: Option<String>,
    pub(crate) model_id: Option<String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentApprovalReasonSummary {
    pub(crate) task_id: String,
    pub(crate) message: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentResultSummary {
    pub(crate) status: String,
    pub(crate) summary: Option<String>,
    pub(crate) error: Option<String>,
    #[serde(default)]
    pub(crate) run_id: Option<String>,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) tool_calls: i64,
    pub(crate) cost_microunits: i64,
    pub(crate) duration_secs: Option<i64>,
    pub(crate) full_result_available: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedAgentTaskSummary {
    pub(crate) id: String,
    pub(crate) stored_step_id: String,
    pub(crate) instruction: String,
    pub(crate) depends_on: Vec<String>,
    #[serde(default)]
    pub(crate) task_kind: WorkflowTaskKind,
    #[serde(default)]
    pub(crate) run_activity: Option<RunActivitySpec>,
    #[serde(default)]
    pub(crate) capabilities: Vec<String>,
    #[serde(default)]
    pub(crate) skill_bindings: Vec<AgentSkillBinding>,
    pub(crate) specialist_id: Option<String>,
    pub(crate) specialist_name: Option<String>,
    pub(crate) executor: AgentExecutorSummary,
    pub(crate) workspace_policy: String,
    #[serde(default)]
    pub(crate) merge_policy: String,
    pub(crate) tools: Vec<String>,
    pub(crate) can_write: bool,
    pub(crate) can_execute: bool,
    pub(crate) can_access_network: bool,
    pub(crate) budget: AgentBudgetProposal,
    pub(crate) timeout_secs: Option<u64>,
    pub(crate) approval_reasons: Vec<String>,
    pub(crate) output_schema: Option<serde_json::Value>,
    pub(crate) result: Option<AgentResultSummary>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentSkillBinding {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) scope: String,
    pub(crate) path: String,
    pub(crate) declared_version: Option<String>,
    pub(crate) skill_md_sha256: String,
    pub(crate) package_id: Option<String>,
    pub(crate) package_version: Option<String>,
    pub(crate) package_source: Option<String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct DynamicAgentWorkflowSummary {
    pub(crate) schema_version: u32,
    pub(crate) approval_policy: AgentApprovalPolicy,
    pub(crate) editable_proposal: DynamicAgentWorkflowProposal,
    pub(crate) tasks: Vec<ResolvedAgentTaskSummary>,
    pub(crate) approval_reasons: Vec<AgentApprovalReasonSummary>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentWorkflowResultDetail {
    pub(crate) workflow_id: String,
    pub(crate) step_id: String,
    pub(crate) attempt: i64,
    pub(crate) status: String,
    pub(crate) response: serde_json::Value,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentWorkflow {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) frame_id: Option<String>,
    #[serde(default)]
    pub(crate) root_workflow_id: String,
    #[serde(default)]
    pub(crate) parent_attempt_id: Option<String>,
    #[serde(default)]
    pub(crate) depth: i64,
    pub(crate) name: String,
    pub(crate) goal: String,
    pub(crate) mode: String,
    pub(crate) status: String,
    pub(crate) max_parallel: i64,
    pub(crate) requires_confirmation: bool,
    pub(crate) version: i64,
    pub(crate) updated_at: i64,
}

#[derive(Deserialize, Clone)]
pub(crate) struct ExecutionContext {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) label: String,
    pub(crate) config_json: String,
    pub(crate) capabilities_json: String,
    pub(crate) last_probe_status: Option<String>,
    pub(crate) last_probe_error: Option<String>,
}

#[derive(Clone, Default)]
pub(crate) struct RuntimeInterpreterForm {
    pub(crate) context_id: String,
    pub(crate) context_label: String,
    pub(crate) context_kind: String,
    pub(crate) python_executable: String,
    pub(crate) rscript_executable: String,
}

impl RuntimeInterpreterForm {
    pub(crate) fn from_context(context: &ExecutionContext) -> Self {
        let config =
            serde_json::from_str::<serde_json::Value>(&context.config_json).unwrap_or_default();
        // When no interpreter is configured explicitly, prefill from the latest
        // probe results so the dialog shows the interpreter actually in use
        // instead of an empty field (issue #651).
        let capabilities = serde_json::from_str::<serde_json::Value>(&context.capabilities_json)
            .unwrap_or_default();
        let string_value = |value: &serde_json::Value, keys: &[&str]| {
            keys.iter()
                .find_map(|key| value.get(*key).and_then(serde_json::Value::as_str))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        let value = |config_keys: &[&str], capability_keys: &[&str]| {
            string_value(&config, config_keys)
                .or_else(|| string_value(&capabilities, capability_keys))
                .unwrap_or_default()
        };
        Self {
            context_id: context.id.clone(),
            context_label: if context.label.trim().is_empty() {
                context.id.clone()
            } else {
                context.label.clone()
            },
            context_kind: context.kind.clone(),
            python_executable: value(
                &["python_executable", "python_path"],
                &["python_executable"],
            ),
            rscript_executable: value(
                &["rscript_executable", "rscript_path"],
                &["rscript_executable"],
            ),
        }
    }
}

#[cfg(test)]
mod runtime_interpreter_form_tests {
    use super::{ExecutionContext, RuntimeInterpreterForm};

    fn context(config_json: &str, capabilities_json: &str) -> ExecutionContext {
        ExecutionContext {
            id: "local".into(),
            kind: "local".into(),
            label: "Local".into(),
            config_json: config_json.into(),
            capabilities_json: capabilities_json.into(),
            last_probe_status: None,
            last_probe_error: None,
        }
    }

    #[test]
    fn prefills_probed_interpreters_when_nothing_is_configured() {
        let form = RuntimeInterpreterForm::from_context(&context(
            "{}",
            r#"{"python_executable":"/opt/conda/bin/python","rscript_executable":"D:\\R-4.5.2\\bin\\Rscript.exe"}"#,
        ));
        assert_eq!(form.python_executable, "/opt/conda/bin/python");
        assert_eq!(form.rscript_executable, r"D:\R-4.5.2\bin\Rscript.exe");
        assert_eq!(form.context_kind, "local");
    }

    #[test]
    fn explicit_configuration_wins_over_probe_results() {
        let form = RuntimeInterpreterForm::from_context(&context(
            r#"{"rscript_executable":"/custom/Rscript"}"#,
            r#"{"rscript_executable":"/probed/Rscript"}"#,
        ));
        assert_eq!(form.rscript_executable, "/custom/Rscript");
    }

    #[test]
    fn blank_configured_values_fall_back_to_probe_results() {
        let form = RuntimeInterpreterForm::from_context(&context(
            r#"{"python_executable":"  "}"#,
            r#"{"python_executable":"/probed/python"}"#,
        ));
        assert_eq!(form.python_executable, "/probed/python");
    }
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
pub(crate) struct TerminalSessionSummary {
    pub(crate) id: String,
    #[serde(rename = "projectId", alias = "project_id")]
    pub(crate) project_id: String,
    #[serde(rename = "contextId", alias = "context_id")]
    pub(crate) context_id: String,
    pub(crate) title: String,
    pub(crate) kind: String,
    #[serde(rename = "displayCwd", alias = "display_cwd")]
    pub(crate) display_cwd: String,
    #[serde(default, rename = "processId", alias = "process_id")]
    pub(crate) process_id: Option<u32>,
    pub(crate) running: bool,
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeKeyDto {
    pub(crate) project_id: String,
    pub(crate) context_id: String,
    pub(crate) language: String,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeInfo {
    pub(crate) runtime_id: String,
    pub(crate) generation: u64,
    pub(crate) key: RuntimeKeyDto,
    pub(crate) status: String,
    pub(crate) interpreter: Option<String>,
    pub(crate) version: Option<String>,
    pub(crate) process_id: Option<u32>,
    pub(crate) started_at_ms: u64,
    pub(crate) last_activity_at_ms: u64,
    pub(crate) resident_memory_bytes: Option<u64>,
    pub(crate) last_error: Option<String>,
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeObject {
    pub(crate) name: String,
    pub(crate) type_name: String,
    pub(crate) summary: String,
    pub(crate) size_bytes: Option<u64>,
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeObjectList {
    pub(crate) objects: Vec<RuntimeObject>,
    pub(crate) total_count: usize,
}

#[derive(Clone, Default)]
pub(crate) struct RuntimeObjectState {
    pub(crate) loading: bool,
    pub(crate) snapshot: Option<RuntimeObjectList>,
    pub(crate) error: Option<String>,
}

#[derive(Clone)]
pub(crate) struct RuntimeSlot {
    pub(crate) project_id: String,
    pub(crate) project_label: String,
    pub(crate) context_id: String,
    pub(crate) context_label: String,
    pub(crate) language: String,
    pub(crate) available: bool,
    pub(crate) info: Option<RuntimeInfo>,
}

/// Mirrors `wisp_store::Run`, minus the columns only the backend acts on
/// (`input_refs_json` / `output_specs_json` / `remote_handle_json` /
/// the always-NULL `script_path`). No blanket `allow(dead_code)`: an unread
/// field here means the UI is dropping data again, and the warning is the
/// whole point.
///
/// Persisted run tool details are now a sanitized subset (id, status, title,
/// exit code, timestamps, output tails) — command, workdir, and environment
/// fields never leave the backend in a persisted payload. Fields that can be
/// absent from that slim shape carry `#[serde(default)]`; the full record
/// still arrives live through `get_run_detail`.
#[derive(Deserialize, Clone, PartialEq)]
pub(crate) struct RunRecord {
    pub(crate) id: String,
    pub(crate) frame_id: Option<String>,
    #[serde(default)]
    pub(crate) context_id: String,
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) kind: String,
    pub(crate) status: String,
    pub(crate) command: Option<String>,
    pub(crate) created_at: i64,
    pub(crate) started_at: Option<i64>,
    pub(crate) ended_at: Option<i64>,
    pub(crate) exit_code: Option<i64>,
    pub(crate) stdout_tail: Option<String>,
    pub(crate) stderr_tail: Option<String>,
    #[serde(rename = "remote_workdir", alias = "remoteWorkdir")]
    pub(crate) remote_workdir: Option<String>,
    pub(crate) timeout_secs: Option<i64>,
    pub(crate) last_polled_at: Option<i64>,
    #[serde(rename = "last_poll_error", alias = "lastPollError")]
    pub(crate) last_poll_error: Option<String>,
    #[serde(default)]
    pub(crate) progress_json: String,
    #[serde(default)]
    pub(crate) env_snapshot_json: String,
}

#[derive(Deserialize, Clone, PartialEq)]
pub(crate) struct RunSummary {
    pub(crate) id: String,
    pub(crate) frame_id: Option<String>,
    pub(crate) context_id: String,
    pub(crate) title: String,
    pub(crate) kind: String,
    pub(crate) status: String,
    pub(crate) created_at: i64,
    pub(crate) started_at: Option<i64>,
    pub(crate) ended_at: Option<i64>,
    pub(crate) exit_code: Option<i64>,
    #[serde(rename = "remote_workdir", alias = "remoteWorkdir")]
    pub(crate) remote_workdir: Option<String>,
    pub(crate) timeout_secs: Option<i64>,
    pub(crate) last_polled_at: Option<i64>,
    #[serde(rename = "last_poll_error", alias = "lastPollError")]
    pub(crate) last_poll_error: Option<String>,
    #[serde(default)]
    pub(crate) progress_json: String,
    #[serde(default)]
    pub(crate) output_fingerprint: String,
}

impl From<&RunRecord> for RunSummary {
    fn from(run: &RunRecord) -> Self {
        Self {
            id: run.id.clone(),
            frame_id: run.frame_id.clone(),
            context_id: run.context_id.clone(),
            title: run.title.clone(),
            kind: run.kind.clone(),
            status: run.status.clone(),
            created_at: run.created_at,
            started_at: run.started_at,
            ended_at: run.ended_at,
            exit_code: run.exit_code,
            remote_workdir: run.remote_workdir.clone(),
            timeout_secs: run.timeout_secs,
            last_polled_at: run.last_polled_at,
            last_poll_error: run.last_poll_error.clone(),
            progress_json: run.progress_json.clone(),
            output_fingerprint: String::new(),
        }
    }
}

#[derive(Deserialize, Clone)]
pub(crate) struct MethodSearchRunState {
    pub(crate) spec_artifact_version_id: String,
    pub(crate) spec_sha256: String,
    pub(crate) control_state: String,
    pub(crate) result_status: Option<String>,
}

#[derive(Deserialize, Clone)]
pub(crate) struct MethodSearchTargetView {
    pub(crate) source_artifact_version_id: String,
    pub(crate) source_path: String,
    pub(crate) symbol: String,
}

#[derive(Deserialize, Clone)]
pub(crate) struct MethodSearchEvaluatorView {
    pub(crate) artifact_version_id: String,
    pub(crate) entry_path: String,
    pub(crate) repetitions: u32,
    pub(crate) timeout_seconds: u64,
}

#[derive(Deserialize, Clone)]
pub(crate) struct MethodSearchGuardrailView {
    pub(crate) metric: String,
    pub(crate) op: String,
    pub(crate) value: f64,
}

#[derive(Deserialize, Clone)]
pub(crate) struct MethodSearchMetricsView {
    pub(crate) primary: String,
    pub(crate) direction: String,
    pub(crate) guardrails: Vec<MethodSearchGuardrailView>,
}

#[derive(Deserialize, Clone)]
pub(crate) struct MethodSearchBudgetView {
    pub(crate) max_candidates: u32,
    pub(crate) max_wall_seconds: u64,
    pub(crate) max_evaluator_seconds: u64,
    pub(crate) max_cost_microunits: u64,
}

#[derive(Deserialize, Clone)]
pub(crate) struct MethodSearchSpecView {
    pub(crate) objective: String,
    pub(crate) target: MethodSearchTargetView,
    pub(crate) evaluator: MethodSearchEvaluatorView,
    pub(crate) metrics: MethodSearchMetricsView,
    pub(crate) protected_paths: Vec<String>,
    pub(crate) constraints: Vec<String>,
    pub(crate) budget: MethodSearchBudgetView,
    pub(crate) final_verification: Option<serde_json::Value>,
}

#[derive(Deserialize, Clone)]
pub(crate) struct BaselineAuditView {
    pub(crate) repetitions: u32,
    pub(crate) successful_repetitions: u32,
    pub(crate) failure_rate: f64,
    pub(crate) median_primary: f64,
    pub(crate) spread: f64,
    pub(crate) noise_floor: f64,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MethodSearchAuditView {
    pub(crate) baseline: BaselineAuditView,
    pub(crate) sentinel_reachable: bool,
    pub(crate) protected_files: Vec<serde_json::Value>,
    pub(crate) target_source_sha256: String,
    pub(crate) findings: Vec<String>,
}

#[derive(Deserialize, Clone)]
pub(crate) struct MethodCandidateView {
    pub(crate) id: String,
    pub(crate) parent_candidate_id: Option<String>,
    pub(crate) sequence: i64,
    pub(crate) strategy_key: String,
    pub(crate) family: String,
    pub(crate) status: String,
    pub(crate) primary_score: Option<f64>,
    pub(crate) runtime_ms: Option<i64>,
    pub(crate) changed_lines: Option<i64>,
    pub(crate) rationale: Option<String>,
    pub(crate) error: Option<String>,
}

#[derive(Deserialize, Clone)]
pub(crate) struct MethodStrategyView {
    pub(crate) strategy_key: String,
    pub(crate) category: String,
    pub(crate) weight: f64,
    pub(crate) attempts: i64,
    pub(crate) improvements: i64,
}

#[derive(Deserialize, Clone)]
pub(crate) struct MethodSearchRunOutput {
    pub(crate) artifact_version_id: String,
    pub(crate) role: String,
    pub(crate) logical_output_key: String,
    pub(crate) source_path: String,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MethodSearchRunDetails {
    pub(crate) run: RunRecord,
    pub(crate) state: MethodSearchRunState,
    pub(crate) spec: MethodSearchSpecView,
    pub(crate) audit: MethodSearchAuditView,
    pub(crate) audit_artifact_version_id: String,
    pub(crate) candidates: Vec<MethodCandidateView>,
    pub(crate) strategies: Vec<MethodStrategyView>,
    pub(crate) outputs: Vec<MethodSearchRunOutput>,
}

#[derive(Deserialize, Clone, Default)]
pub(crate) struct MethodSearchProgressView {
    #[serde(default)]
    pub(crate) phase: String,
    pub(crate) baseline_primary: Option<f64>,
    pub(crate) best_primary: Option<f64>,
    #[serde(default)]
    pub(crate) candidate_count: usize,
    #[serde(default)]
    pub(crate) successful_count: usize,
    #[serde(default)]
    pub(crate) failed_count: usize,
    #[serde(default)]
    pub(crate) cost_microunits: u64,
    pub(crate) current_strategy: Option<String>,
    pub(crate) last_checkpoint_at: Option<i64>,
    pub(crate) best_candidate_id: Option<String>,
}

#[derive(Deserialize, Clone)]
pub(crate) struct RunProgress {
    pub(crate) phase: String,
    pub(crate) direction: String,
    pub(crate) completed_bytes: u64,
    pub(crate) total_bytes: u64,
    pub(crate) files_completed: u64,
    pub(crate) files_total: u64,
    pub(crate) current_file: Option<String>,
    pub(crate) bytes_per_second: Option<u64>,
    pub(crate) eta_seconds: Option<u64>,
    pub(crate) updated_at: i64,
}

/// Provenance for a produced file — mirrors the `get_artifact_provenance`
/// Tauri command output (src-tauri `ArtifactProvenance`). Deserialize only.
#[derive(Clone, Deserialize, Default)]
pub(crate) struct ArtifactProvenance {
    pub(crate) code: String,
    pub(crate) language: String,
    pub(crate) output: String,
    #[serde(default)]
    pub(crate) inputs: Vec<ProvInput>,
    pub(crate) env: Option<ProvEnv>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct ProvInput {
    pub(crate) path: String,
    pub(crate) produced_here: bool,
}

#[derive(Clone, Deserialize)]
pub(crate) struct ProvEnv {
    #[serde(default)]
    pub(crate) packages: Vec<ProvPkg>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct ProvPkg {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) version: String,
}
