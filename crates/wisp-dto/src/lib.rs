//! Shared data model for the wisp UI ⇄ Tauri backend boundary: the serde DTOs
//! exchanged over `invoke`/events plus the UI's in-memory view/form types.
//!
//! This crate holds *data only* — struct/enum shapes and trivial inherent
//! impls (defaults, conversions, small classifiers). It must not depend on
//! Leptos reactivity, JS bindings, Tauri, or view code, so it compiles for both
//! wasm32 (UI) and native (backend contract tests). Behaviour that needs i18n,
//! signals, or FFI lives in the consuming crates.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Deserialize, Serialize, Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub struct ContextUsage {
    #[serde(default)]
    pub system_prompt: usize,
    #[serde(default)]
    pub tool_definitions: usize,
    #[serde(default)]
    pub rules: usize,
    #[serde(default)]
    pub skills: usize,
    #[serde(default)]
    pub mcp_dynamic_tools: usize,
    #[serde(default)]
    pub subagent_definitions: usize,
    #[serde(default)]
    pub conversation: usize,
}

impl ContextUsage {
    pub fn total(self) -> usize {
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
pub struct ContextUsageSnapshot {
    pub used: usize,
    pub max: usize,
    pub breakdown: Option<ContextUsage>,
    pub estimated: bool,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextToolDetail {
    pub name: String,
    pub description: String,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextUsageDetails {
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub tool_definitions: Vec<ContextToolDetail>,
    #[serde(default)]
    pub rules: String,
    #[serde(default)]
    pub skills: String,
    #[serde(default)]
    pub mcp_dynamic_tools: Vec<ContextToolDetail>,
    #[serde(default)]
    pub subagent_definitions: Vec<ContextToolDetail>,
}

/// Progress emitted by the native project archive importer/exporter. Mirrors
/// `ProjectTransferProgress` in `src-tauri/src/project_transfer.rs`.
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectTransferProgress {
    pub direction: String,
    pub stage: String,
    #[serde(default)]
    pub project_id: Option<String>,
    pub completed_files: u64,
    pub total_files: Option<u64>,
    pub completed_bytes: u64,
    pub total_bytes: Option<u64>,
    #[serde(default)]
    pub current_path: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

impl ProjectTransferProgress {
    pub fn selecting(direction: &str, project_id: Option<String>) -> Self {
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

    pub fn is_complete(&self) -> bool {
        self.stage == "complete"
    }

    pub fn is_failed(&self) -> bool {
        self.stage == "failed"
    }

    pub fn is_active(&self) -> bool {
        !self.is_complete() && !self.is_failed()
    }

    pub fn is_exporting_project(&self, project_id: &str) -> bool {
        self.is_active()
            && self.direction == "export"
            && self.project_id.as_deref() == Some(project_id)
    }

    pub fn complete(
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

    pub fn failed(direction: &str, project_id: Option<String>, error: String) -> Self {
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
pub struct CustomCredentialStatus {
    pub id: String,
    pub name: String,
    pub env_var: String,
    pub present: bool,
}

#[derive(Deserialize, Clone, Debug, Hash, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MessageResource {
    pub id: String,
    pub ordinal: i64,
    pub original_reference: String,
    pub artifact_id: Option<String>,
    pub artifact_version_id: Option<String>,
    pub display_name: String,
    pub kind: String,
    pub mime_type: String,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Deserialize, Clone)]
#[serde(tag = "kind")]
pub enum AgentEvent {
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
pub struct ReviewFinding {
    #[serde(default)]
    pub message_index: usize,
    #[serde(default)]
    pub claim: String,
    #[serde(default)]
    pub evidence: String,
    #[serde(default)]
    pub fix: String,
    #[serde(default)]
    pub verdict: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub status: String,
}

#[derive(Deserialize, Clone, Hash, PartialEq, Eq)]
pub struct ReviewReport {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<ReviewFinding>,
    #[serde(default)]
    pub reviewer_model: String,
    #[serde(default)]
    pub reviewer_effort: String,
    #[serde(default)]
    pub reviewer_backend: String,
    #[serde(default)]
    pub review_status: String,
    #[serde(default = "default_evidence_coverage")]
    pub evidence_coverage: u8,
    #[serde(default)]
    pub coverage_gaps: Vec<String>,
}

fn default_evidence_coverage() -> u8 {
    100
}

#[derive(Clone)]
pub enum ChatItem {
    User(String),
    /// A user turn queued (#433) while the same session is still running. It
    /// waits, cancellable, until the backend drains it into a fresh
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
    BranchMerge {
        text: String,
        branch_id: String,
        branch_title: String,
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
pub struct SideChatEvidence {
    pub source_id: String,
    #[serde(default)]
    pub event_seq: Option<i64>,
    #[serde(default)]
    pub message_seq: Option<i64>,
    pub turn: usize,
    pub role: String,
    pub excerpt: String,
    #[serde(default)]
    pub relevance: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SideChatResponse {
    pub answer: String,
    pub snapshot_version: i64,
    #[serde(default)]
    pub evidence: Vec<SideChatEvidence>,
    #[serde(default)]
    pub no_evidence: bool,
}

#[derive(Clone)]
pub enum SideChatItem {
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
pub struct TurnUndoPreview {
    #[serde(default)]
    pub restore_files: Vec<String>,
    #[serde(default)]
    pub remove_files: Vec<String>,
    #[serde(default)]
    pub remove_artifacts: Vec<String>,
    #[serde(default)]
    pub unsupported_files: Vec<String>,
    #[serde(default)]
    pub conflicts: Vec<String>,
}

#[derive(Clone)]
pub struct TurnUndoDialog {
    pub session_id: String,
    pub user_index: usize,
    pub user_ui_index: usize,
    pub draft: String,
    pub preview: TurnUndoPreview,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub enum ReviewTransitionPhase {
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
    pub fn fingerprint(&self) -> u64 {
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
            Self::BranchMerge {
                text,
                branch_id,
                branch_title,
            } => {
                (15u8, branch_id, branch_title).hash(&mut h);
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
                ..
            } => {
                (4u8, name, ok, duration_ms).hash(&mut h);
                hash_text_sampled(&mut h, input);
                hash_text_sampled(&mut h, output);
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
pub struct PlanEntry {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub status: PlanStatus,
    #[serde(default)]
    pub priority: PlanPriority,
}

/// `from = "String"` makes deserialization total: an agent that invents a
/// status ("blocked", "skipped") degrades to the default instead of failing the
/// whole card. Serialization still writes the ACP spelling.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
#[serde(rename_all = "snake_case", from = "String")]
pub enum PlanStatus {
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
pub enum PlanPriority {
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
pub const PROPOSE_PLAN_TOOL: &str = "propose_plan";

/// Who produced the plan: the ACP agent's own plan updates, or the built-in
/// `propose_plan` tool. Both render through the same card.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
#[serde(rename_all = "snake_case", from = "String")]
pub enum PlanSource {
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
pub enum PlanState {
    Streaming,
    #[default]
    Ready,
}

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq)]
pub struct PlanCard {
    pub entries: Vec<PlanEntry>,
    pub source: PlanSource,
    pub state: PlanState,
}

/// Parses both the live ACP `plan` payload and the persisted plan body — they
/// carry the same `{ source?, entries[] }` shape on purpose. Foreign JSON, so
/// every field is optional and unknown values fall back to the defaults.
pub fn parse_plan_card(payload: &serde_json::Value) -> PlanCard {
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
pub const ASK_USER_TOOL: &str = "ask_user";

#[derive(Serialize, Deserialize, Clone, Debug, Default, Hash, PartialEq, Eq)]
pub struct QuestionOption {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub description: String,
}

/// Card lifecycle. `Answered` is never persisted for the built-in source — a
/// question counts as answered once a later user message exists (the answer IS
/// that message). The ACP source persists `expired` for pendings that can no
/// longer be resolved (the bridge process died with them).
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum QuestionState {
    #[default]
    Pending,
    Answered,
    Expired,
}

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq)]
pub struct QuestionCard {
    pub question: String,
    pub options: Vec<QuestionOption>,
    pub allow_freeform: bool,
    pub source: PlanSource,
    /// Present only for the ACP source: the pending id `respond_ask_user` resolves.
    pub request_id: Option<String>,
    pub state: QuestionState,
}

/// Parses the `ask_user` tool body, the live ACP request payload, and the
/// reloaded row — all carry the same `{ question, options[], allow_freeform }`
/// shape; the ACP reload row adds `request_id` and `status`. Foreign JSON, so
/// every field is optional and junk degrades instead of failing the card.
pub fn parse_question_card(payload: &serde_json::Value) -> QuestionCard {
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
pub fn settle_question_cards(items: &mut [ChatItem]) {
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

pub fn active_model_label(models: &[ModelProfile]) -> Option<String> {
    model_label(models, None)
}

pub fn model_label(models: &[ModelProfile], model_id: Option<&str>) -> Option<String> {
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

pub fn session_model_label(
    models: &[ModelProfile],
    session_models: &HashMap<String, String>,
    session_id: Option<&str>,
) -> Option<String> {
    model_label(
        models,
        session_id.and_then(|session_id| session_models.get(session_id).map(String::as_str)),
    )
}

/// The profile a session resolves to right now: bound profile → active →
/// first chat model. Shared by the session-scoped getters.
pub fn session_profile<'a>(
    models: &'a [ModelProfile],
    session_models: &HashMap<String, String>,
    session_id: Option<&str>,
) -> Option<&'a ModelProfile> {
    let bound = session_id.and_then(|id| session_models.get(id));
    models
        .iter()
        .find(|model| model.is_chat_model() && bound == Some(&model.id))
        .or_else(|| {
            models
                .iter()
                .find(|model| model.active && model.is_chat_model())
        })
        .or_else(|| models.iter().find(|model| model.is_chat_model()))
}

/// Mirrors `models::effective_context_window` in src-tauri: a configured
/// window below 4K counts as "unset" and falls back to 128K.
pub fn effective_context_window(value: u64) -> u64 {
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
pub fn session_context_window(
    models: &[ModelProfile],
    session_models: &HashMap<String, String>,
    session_id: Option<&str>,
) -> Option<u64> {
    let bound = session_id.and_then(|session_id| session_models.get(session_id));
    if bound.is_some_and(|id| id.starts_with("acp:")) {
        return None;
    }
    session_profile(models, session_models, session_id)
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
            endpoint_suffix: String::new(),
            model: format!("model-{id}"),
            has_api_key: true,
            active,
            max_tokens: 8_192,
            context_window,
            reasoning_effort: String::new(),
            service_tier: String::new(),
            supports_vision: false,
            use_for_vision: false,
            use_for_image_generation: false,
            image_size: String::new(),
            image_quality: String::new(),
            image_aspect_ratio: String::new(),
            image_resolution: String::new(),
            use_for_video_generation: false,
            video_duration_secs: None,
            video_aspect_ratio: None,
            video_resolution: None,
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
pub struct PreviewSelection {
    pub text: String,
    #[serde(default)]
    pub path: String,
    pub x: i32,
    pub y: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionAttach {
    pub path: String,
    #[serde(default)]
    pub jump_to_chat: bool,
}

/// Detail of the `wisp:pins-ask-ai` event: image comment pins assembled into
/// one composer message by the preview. Serialized as a struct (not
/// `serde_json::json!`) so serde-wasm-bindgen emits a plain JS object — a
/// `Value::Object` would become an ES Map the listener cannot deserialize.
#[derive(Serialize, Deserialize)]
pub struct PinsAskAi {
    pub path: String,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct ArtifactInfo {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub path: String,
    #[serde(default)]
    pub location: Option<String>,
    pub ts: i64,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub project_name: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub session_title: Option<String>,
    #[serde(default)]
    pub size_bytes: Option<i64>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub logical_path: Option<String>,
    #[serde(default)]
    pub source_discarded: bool,
}

/// Immutable item in the app-global library database. Source names are
/// snapshots, so this remains useful after its project or session is deleted.
#[derive(Deserialize, Clone, PartialEq)]
pub struct LibraryItem {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub language: Option<String>,
    #[serde(default)]
    pub code: Rc<str>,
    pub content_type: Option<String>,
    pub source_project_id: String,
    pub source_project_name: String,
    pub source_session_id: String,
    pub source_session_title: String,
    pub source_path: Option<String>,
    pub created_at: i64,
}

impl LibraryItem {
    pub fn matches_code(&self, session: &str, language: &str, code: &str) -> bool {
        self.kind == "code"
            && self.source_session_id == session
            && self.language.as_deref().unwrap_or_default() == language
            && self.code.as_ref() == code
    }
}

/// Bounded Library list row. Full code/text is fetched only for the active
/// session or an opened detail.
#[derive(Deserialize, Clone, PartialEq)]
pub struct LibraryItemSummary {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub language: Option<String>,
    #[serde(default)]
    pub code_preview: String,
    pub source_project_id: String,
    pub source_project_name: String,
    pub source_session_id: String,
    pub source_session_title: String,
    pub source_path: Option<String>,
    pub created_at: i64,
}

impl LibraryItemSummary {
    pub fn matches_figure(&self, session: &str, path: &str) -> bool {
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
pub struct LibraryItemDetail {
    #[serde(flatten)]
    pub item: LibraryItem,
    pub base64: Option<String>,
}

/// One immutable version of a library item's code — mirrors the wisp-store
/// `LibraryItemVersion` returned by `list_library_item_versions` /
/// `update_library_code`. Version 1 is the original snapshot (`id` equals the
/// item id); higher numbers are user edits.
#[derive(Deserialize, Clone, PartialEq)]
pub struct LibraryItemVersion {
    pub id: String,
    pub item_id: String,
    pub version_number: i64,
    pub parent_version_id: Option<String>,
    pub language: Option<String>,
    #[serde(default)]
    pub code: String,
    pub origin: String,
    pub created_at: i64,
}

#[derive(Deserialize, Clone, PartialEq)]
pub struct SessionSearchInfo {
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    pub title: String,
    #[serde(default)]
    pub ts: i64,
    #[serde(default)]
    pub activity_at: i64,
    #[serde(default)]
    pub status: String,
}

#[derive(Serialize, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComposerReferenceArg {
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
pub struct SshHost {
    pub alias: String,
    /// Real address (IP or domain) for manually created hosts; when absent
    /// the alias itself is the target, resolved via ~/.ssh/config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// `key` (default) or `password`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    /// Whether a password is stored in the OS keyring (never the password itself).
    #[serde(default)]
    pub has_password: bool,
    /// Write-only password from the form; never returned by list APIs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

/// Mirrors the `get_storage_usage` payload built in
/// src-tauri/src/settings_commands.rs — align field by field on both sides.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct StorageEntry {
    pub key: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ProjectStorageUsage {
    pub id: String,
    pub name: String,
    pub path: String,
    pub bytes: u64,
}

/// Mirrors `configure::AppearancePrefsView` in src-tauri/src/configure.rs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AppearancePrefs {
    #[serde(default = "default_theme_mode")]
    pub theme: String,
    #[serde(default = "default_light_palette")]
    pub light_palette: String,
    #[serde(default = "default_dark_palette")]
    pub dark_palette: String,
    #[serde(default = "default_ui_font_size")]
    pub ui_font_size: u16,
    #[serde(default = "default_code_font_size")]
    pub code_font_size: u16,
    #[serde(default)]
    pub ui_font_family: String,
    #[serde(default)]
    pub code_font_family: String,
    #[serde(default = "default_true")]
    pub selection_popup_enabled: bool,
    #[serde(default)]
    pub send_with_modifier: bool,
    #[serde(default)]
    pub custom_css: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AppearancePrefsView {
    #[serde(default)]
    pub saved: bool,
    #[serde(flatten)]
    pub prefs: AppearancePrefs,
}

fn default_theme_mode() -> String {
    "system".into()
}
fn default_light_palette() -> String {
    "paper".into()
}
fn default_dark_palette() -> String {
    "charcoal".into()
}
fn default_ui_font_size() -> u16 {
    14
}
fn default_code_font_size() -> u16 {
    12
}
fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct StorageUsage {
    pub data_dir: String,
    #[serde(default)]
    pub projects: Vec<ProjectStorageUsage>,
    #[serde(default)]
    pub entries: Vec<StorageEntry>,
    pub total_bytes: u64,
}

/// Mirrors the token-usage payloads in crates/wisp-store/src/sessions.rs and
/// src-tauri/src/settings_commands.rs — align field by field on both sides.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ProjectTokenUsage {
    pub project_id: String,
    pub name: String,
    pub workspace_dir: String,
    pub updated_at: i64,
    pub session_count: i64,
    pub input: i64,
    pub output: i64,
    pub reasoning: i64,
    pub cached: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct SessionTokenUsage {
    pub id: String,
    pub title: String,
    pub updated_at: i64,
    pub input: i64,
    pub output: i64,
    pub reasoning: i64,
    pub cached: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct SessionTokenUsagePage {
    #[serde(default)]
    pub items: Vec<SessionTokenUsage>,
    pub total: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct TokenUsageDay {
    pub date: String,
    pub tokens: i64,
    #[serde(default)]
    pub future: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ModelTokenUsage {
    pub model: String,
    pub tokens: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ToolCallUsage {
    pub kind: String,
    pub name: String,
    pub calls: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct TokenUsageOverview {
    #[serde(default)]
    pub workspaces: Vec<ProjectTokenUsage>,
    #[serde(default)]
    pub days: Vec<TokenUsageDay>,
    #[serde(default)]
    pub models: Vec<ModelTokenUsage>,
    #[serde(default)]
    pub tools: Vec<ToolCallUsage>,
}

/// Mirrors `SshTrustEdge` in src-tauri/src/run_context/transfer.rs — align
/// field by field on both sides.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct SshTrustEdge {
    pub source_context_id: String,
    pub destination_context_id: String,
    pub destination_target: String,
    #[serde(default)]
    pub destination_port: Option<u16>,
    #[serde(default)]
    pub key_path: Option<String>,
    pub managed: bool,
    pub verified_at: i64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RevokeTrustResponse {
    pub edges: Vec<SshTrustEdge>,
    #[serde(default)]
    pub cleanup_error: Option<String>,
}

#[derive(Clone)]
pub enum ComposerAttachment {
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
pub struct UploadFileResult {
    pub ok: bool,
    pub info: Option<ArtifactInfo>,
    pub filename: Option<String>,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    pub provider: String,
    pub api_url: String,
    pub model: String,
    #[serde(default)]
    pub label: String,
    pub has_api_key: bool,
    #[serde(default)]
    pub locale: String,
    #[serde(default)]
    pub workspace_dir: String,
    #[serde(default = "default_max_iter")]
    pub max_iter: i64,
    #[serde(default = "default_auto_compact")]
    pub auto_compact: bool,
    #[serde(default)]
    pub auto_continue: bool,
    #[serde(default = "default_auto_continue_limit")]
    pub auto_continue_limit: u64,
    #[serde(default = "default_follow_up_questions")]
    pub follow_up_questions: bool,
    #[serde(default = "default_resume_last_session")]
    pub resume_last_session: bool,
    #[serde(default)]
    pub max_tokens: u64,
    #[serde(default)]
    pub reasoning_effort: String,
    #[serde(default)]
    pub service_tier: String,
    #[serde(default)]
    pub proxy_url: String,
    #[serde(default)]
    pub supports_vision: bool,
    #[serde(default = "default_sync_backend")]
    pub sync_backend: String,
    #[serde(default)]
    pub sync_relay_url: String,
    #[serde(default)]
    pub sync_folder: String,
    #[serde(default)]
    pub sync_relay_token: String,
    #[serde(default)]
    pub has_sync_relay_token: bool,
    #[serde(default)]
    pub pet_enabled: bool,
    #[serde(default)]
    pub pet_directory: String,
    #[serde(default = "default_notifications_enabled")]
    pub notifications_enabled: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserUrlFilterRule {
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserUrlFilters {
    #[serde(default)]
    pub block: Vec<BrowserUrlFilterRule>,
    #[serde(default)]
    pub prefer: Vec<BrowserUrlFilterRule>,
}

/// One tab Wisp opened during a conversation turn, offered for cleanup.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserTabCleanupItem {
    #[serde(default)]
    pub session: String,
    pub tab_id: i64,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub initial_url: String,
}

/// Prompt to close tabs Wisp opened in one turn. `tab_id` stays valid across
/// in-tab navigations; only this turn's ids are included.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserTabCleanupPrompt {
    pub turn_id: String,
    pub frame_id: String,
    #[serde(default)]
    pub tabs: Vec<BrowserTabCleanupItem>,
}

/// A tab whose current page needs the user to complete a human-verification
/// challenge before browser automation may continue.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserNeedsHumanTab {
    #[serde(default)]
    pub session: String,
    pub tab_id: i64,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub frame_id: String,
    #[serde(default)]
    pub turn_id: String,
}

/// Live set of tabs waiting on human verification. Replaces the previous
/// snapshot each time it is emitted.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserNeedsHumanPrompt {
    #[serde(default)]
    pub tabs: Vec<BrowserNeedsHumanTab>,
}

/// Result of re-scanning tabs the user marked as completed.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserNeedsHumanConfirmResult {
    #[serde(default)]
    pub still_required: Vec<BrowserNeedsHumanTab>,
    #[serde(default)]
    pub cleared: Vec<BrowserNeedsHumanTab>,
}

/// Reply of `open_browser_extension_page`: managed extension path and whether
/// a browser was launched on its extension-manager page.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserExtensionSetup {
    #[serde(default)]
    pub extension_path: Option<String>,
    #[serde(default)]
    pub opened: bool,
}

/// Compatibility and integrity state of the shared-browser extension.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserExtensionStatus {
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub current_version: Option<String>,
    #[serde(default)]
    pub bundled_version: Option<String>,
    #[serde(default)]
    pub current_protocol: i64,
    #[serde(default)]
    pub required_protocol: i64,
    #[serde(default)]
    pub update_required: bool,
    #[serde(default)]
    pub automatic_reload_available: bool,
    #[serde(default)]
    pub extension_path: Option<String>,
    #[serde(default)]
    pub extension_path_verified: bool,
    #[serde(default)]
    pub integrity_verified: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// Result of preparing the bundled browser extension and attempting to reload
/// it. `outcome` is `updated`, `manual_reload_required`, or `error`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserExtensionUpdateResult {
    #[serde(default)]
    pub outcome: String,
    #[serde(default)]
    pub status: BrowserExtensionStatus,
    #[serde(default)]
    pub opened: bool,
    #[serde(default)]
    pub error: Option<String>,
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

fn default_auto_continue_limit() -> u64 {
    10
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
pub struct ChannelsStatus {
    #[serde(default)]
    pub feishu_enabled: bool,
    #[serde(default)]
    pub feishu_bound: bool,
    #[serde(default)]
    pub feishu_international: bool,
    #[serde(default)]
    pub feishu_app_id: String,
    #[serde(default)]
    pub feishu_has_secret: bool,
    #[serde(default)]
    pub feishu_state: String,
    #[serde(default)]
    pub feishu_detail: String,
    #[serde(default)]
    pub feishu_owner_open_id: String,
    #[serde(default)]
    pub feishu_pending_owner_open_id: String,
    #[serde(default)]
    pub weixin_enabled: bool,
    #[serde(default)]
    pub weixin_bound: bool,
    #[serde(default)]
    pub weixin_state: String,
    #[serde(default)]
    pub weixin_detail: String,
    #[serde(default)]
    pub device: DeviceBridgeStatus,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeviceBridgeStatus {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_device_bridge_mode")]
    pub mode: String,
    #[serde(default)]
    pub has_token: bool,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub bind_ipv4: String,
    #[serde(default = "default_device_bridge_port")]
    pub port: u16,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub detail: String,
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
pub struct WeixinBindStart {
    pub qrcode: String,
    pub qr_image: String,
}

/// Mirrors the opaque Feishu OAuth device-flow DTOs from `src-tauri`.
#[derive(Deserialize, Clone)]
pub struct FeishuBindStart {
    pub flow_id: String,
    pub qr_image: String,
    pub expires_in_seconds: u64,
}

#[derive(Deserialize, Clone)]
pub struct FeishuBindPoll {
    pub state: String,
    pub retry_after_ms: u64,
    pub app_id: String,
}

fn default_max_iter() -> i64 {
    100
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: "openai".into(),
            api_url: "https://api.deepseek.com".into(),
            model: "deepseek-v4-flash".into(),
            label: "deepseek-v4-flash".into(),
            has_api_key: false,
            locale: "en".into(),
            workspace_dir: String::new(),
            max_iter: default_max_iter(),
            auto_compact: true,
            auto_continue: false,
            auto_continue_limit: default_auto_continue_limit(),
            follow_up_questions: true,
            resume_last_session: true,
            max_tokens: 8192,
            reasoning_effort: String::new(),
            service_tier: String::new(),
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
pub struct PetStatus {
    pub enabled: bool,
    pub directory: String,
    pub asset: Option<PetAsset>,
    pub error: Option<String>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PetAsset {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub sprite_version_number: u8,
    pub spritesheet_data_url: String,
    pub frame_counts: std::collections::BTreeMap<String, u8>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSyncResult {
    pub direction: String,
    pub uploaded_files: usize,
    pub downloaded_files: usize,
    pub skipped_paths: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DemoInfo {
    pub id: String,
    pub title: String,
}

#[derive(Deserialize, Clone)]
pub struct Demo {
    pub title: String,
    pub request: String,
    pub response: String,
    pub thinking: Option<String>,
    #[serde(default)]
    pub items: Vec<LoadedItem>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageArgs {
    // Tauri v2 maps JS camelCase keys to snake_case params; the JS side must
    // send `sessionId` or the backend sees `None` and forks a new conversation.
    pub session_id: Option<String>,
    pub message: String,
    #[serde(default)]
    pub attachments: Vec<String>,
    #[serde(default)]
    pub references: Vec<ComposerReferenceArg>,
    #[serde(default)]
    pub resume: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acp_agent_id: Option<String>,
    /// Guide (#410): inject into the running turn's next loop iteration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guide: Option<bool>,
    /// Guide (#410): roll back the interrupted turn before sending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace: Option<bool>,
}

/// Queue (#433): park a follow-up turn behind the running one.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueTurnArgs {
    pub session_id: String,
    pub id: u64,
    pub message: String,
    #[serde(default)]
    pub attachments: Vec<String>,
    #[serde(default)]
    pub references: Vec<ComposerReferenceArg>,
}

/// Queue (#433): edit / cancel / cut-in a parked follow-up by id.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedTurnActionArgs {
    pub session_id: String,
    pub id: u64,
    pub action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcpAgentProfile {
    pub id: String,
    pub label: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AcpAgentInfo {
    #[serde(default)]
    pub protocol_version: u16,
    #[serde(default)]
    pub implementation: Option<serde_json::Value>,
    #[serde(default)]
    pub capabilities: serde_json::Value,
    #[serde(default)]
    pub auth_methods: Vec<AcpAuthMethod>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcpAuthMethod {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpSessionUpdate {
    pub frame_id: String,
    pub kind: String,
    #[serde(default)]
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpSessionState {
    pub frame_id: String,
    #[serde(default)]
    pub modes: Option<serde_json::Value>,
    #[serde(default)]
    pub config_options: Option<Vec<serde_json::Value>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpPermissionResolved {
    pub frame_id: String,
    pub request_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct AcpPermissionOption {
    pub id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpPermissionRequest {
    pub request_id: String,
    pub frame_id: String,
    #[serde(default)]
    pub tool_call: serde_json::Value,
    #[serde(default)]
    pub options: Vec<AcpPermissionOption>,
}

/// `ask-user-request`: an ACP agent's bridge `ask_user` call waiting for the
/// user. `payload` is the tool body `parse_question_card` reads.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskUserRequest {
    pub request_id: String,
    pub frame_id: String,
    #[serde(default)]
    pub payload: serde_json::Value,
}

/// `ask-user-resolved`: the pending question was answered (or expired with the
/// turn) and its card should settle.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskUserResolved {
    pub request_id: String,
    pub frame_id: String,
    #[serde(default)]
    pub expired: bool,
}

#[derive(Deserialize, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub ts: i64,
    #[serde(default)]
    pub folder_id: Option<String>,
    /// Source session this one was branched from; nested under it in the sidebar.
    #[serde(default)]
    pub branched_from: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub branch_state: Option<String>,
    /// Persisted system prompt lags AGENTS.md / WISP.md; sidebar offers reload.
    #[serde(default)]
    pub stale_prompt: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionBranchDeltaMessage {
    pub seq: i64,
    pub role: String,
    pub text: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionBranchLink {
    pub id: String,
    pub title: String,
    pub source_session_id: String,
    pub checkpoint_user_index: usize,
    pub checkpoint_kind: String,
    #[serde(default)]
    pub merged: bool,
    #[serde(default)]
    pub merge_summary: Option<String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionBranchMergePreview {
    pub main_session_id: String,
    pub branch_session_id: String,
    pub branch_title: String,
    pub checkpoint_user_index: usize,
    pub checkpoint_kind: String,
    pub guard_hash: String,
    pub new_message_count: usize,
    pub messages: Vec<SessionBranchDeltaMessage>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionBranchMerge {
    pub main_session_id: String,
    pub branch_session_id: String,
    pub summary_message_seq: i64,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Exploration {
    pub id: String,
    pub checkpoint_id: String,
    pub frame_id: String,
    pub name: String,
    pub status: String,
    pub workspace_dir: String,
    pub workspace_backend: String,
    pub scope_generation: i64,
    pub warnings_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ExplorationSummary {
    pub exploration: Exploration,
    pub source_frame_id: String,
    pub checkpoint_user_index: usize,
    pub isolation_summary_json: String,
}

impl ExplorationSummary {
    pub fn isolation_is_full(&self) -> bool {
        serde_json::from_str::<serde_json::Value>(&self.isolation_summary_json)
            .ok()
            .and_then(|value| value.get("partial").and_then(serde_json::Value::as_bool))
            != Some(true)
            && serde_json::from_str::<Vec<serde_json::Value>>(&self.exploration.warnings_json)
                .map_or(true, |warnings| warnings.is_empty())
    }
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ExplorationFileDelta {
    pub path: String,
    pub kind: String,
    #[serde(default)]
    pub before: Option<serde_json::Value>,
    #[serde(default)]
    pub after: Option<serde_json::Value>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExplorationArtifactDelta {
    pub logical_key: String,
    pub before_artifact_id: Option<String>,
    pub before_version_id: Option<String>,
    pub after_artifact_id: String,
    pub after_version_id: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ExplorationExternalResource {
    pub id: String,
    pub kind: String,
    pub uri: String,
    pub version: Option<String>,
    pub checksum: Option<String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ExplorationEffect {
    pub id: String,
    pub effect_kind: String,
    pub recoverability: String,
    pub target_summary: String,
    pub metadata_json: String,
    pub created_at: i64,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExplorationDiff {
    pub exploration_id: String,
    pub files: Vec<ExplorationFileDelta>,
    pub artifacts: Vec<ExplorationArtifactDelta>,
    pub runs: Vec<RunRecord>,
    pub decisions: Vec<ResearchNode>,
    pub research_edges: Vec<ResearchEdge>,
    pub external_resources: Vec<ExplorationExternalResource>,
    pub external_effects: Vec<ExplorationEffect>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExplorationMainlineChanges {
    pub files: Vec<ExplorationFileDelta>,
    pub artifact_keys: Vec<String>,
    pub entity_keys: Vec<String>,
    pub source_message_head: i64,
    pub source_ui_event_head: i64,
    pub state_generation: i64,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PromotionBlocker {
    pub code: String,
    pub message: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PromotionEligibility {
    pub eligible: bool,
    pub code: Option<String>,
    pub reasons: Vec<PromotionBlocker>,
    pub expected_guard_hash: String,
    pub manual_resolution_available: bool,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExplorationPromotionPreview {
    pub exploration: Exploration,
    pub diff: ExplorationDiff,
    pub mainline_changes: ExplorationMainlineChanges,
    pub eligibility: PromotionEligibility,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExplorationPromotionResult {
    pub mainline_frame_id: String,
}

/// One Codex CLI or Claude Code conversation offered by the import modal.
/// `state` is "new" | "imported" | "updatable".
#[derive(Deserialize, Clone, PartialEq)]
pub struct ExternalSessionInfo {
    pub path: String,
    pub session_id: String,
    pub title: String,
    pub cwd: String,
    pub message_count: usize,
    pub last_active_at: i64,
    pub state: String,
}

#[derive(Deserialize, Clone, PartialEq)]
pub struct ExternalSessionPreviewLine {
    pub role: String,
    pub text: String,
}

#[derive(Deserialize, Clone, Default)]
pub struct ExternalImportSummary {
    pub imported: usize,
    pub updated: usize,
    pub skipped: usize,
    pub failed: usize,
    #[serde(default)]
    pub synced_paths: Vec<String>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct SessionCursor {
    pub ts: i64,
    pub id: String,
}

#[derive(Deserialize)]
pub struct SessionPage {
    pub items: Vec<SessionInfo>,
    pub next_cursor: Option<SessionCursor>,
    pub running_ids: Vec<String>,
}

#[derive(Deserialize, Clone)]
pub struct FolderInfo {
    pub id: String,
    pub name: String,
}

/// A transcript row returned by `load_session`.
#[derive(Deserialize, Clone)]
pub struct LoadedItem {
    pub role: String,
    pub text: String,
    pub tool_name: Option<String>,
    pub ok: Option<bool>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub model_name: Option<String>,
    #[serde(default)]
    pub call_id: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub locations: Option<String>,
    #[serde(default)]
    pub resources: Vec<MessageResource>,
}

#[derive(Deserialize)]
pub struct LoadedSessionPage {
    pub items: Vec<LoadedItem>,
    pub next_before_seq: Option<i64>,
    pub user_offset: usize,
    #[serde(default)]
    pub outline: Vec<SessionOutlineItem>,
    #[serde(default)]
    pub presentations: Vec<LoadedPresentation>,
    #[serde(default)]
    pub branches: Vec<SessionBranchLink>,
    #[serde(default)]
    pub branch_state: Option<String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionOutlineItem {
    pub user_index: usize,
    #[serde(default)]
    pub seq: Option<i64>,
    pub text: String,
    #[serde(default)]
    pub sent_at: Option<i64>,
    #[serde(default)]
    pub response_at: Option<i64>,
}

#[derive(Deserialize, Clone)]
pub struct LoadedPresentation {
    #[serde(default)]
    pub presentation_id: String,
    pub presentation_kind: String,
    pub payload: serde_json::Value,
}

#[derive(Clone, Copy, Default)]
pub struct TranscriptPageState {
    pub next_before_seq: Option<i64>,
    pub user_offset: usize,
    pub loading: bool,
    pub window_user_start: usize,
}

impl LoadedItem {
    pub fn into_chat(self) -> ChatItem {
        match self.role.as_str() {
            "user" => ChatItem::User(self.text),
            "branch_merge" => ChatItem::BranchMerge {
                text: self.text,
                branch_id: self.input,
                branch_title: self.tool_name.unwrap_or_default(),
            },
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
pub struct TableData {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Clone, PartialEq)]
pub enum PreviewData {
    Table(Rc<TableData>),
    Latex { tex: String, display: bool },
    File { path: String, kind: String },
    Fasta(Rc<str>),
}

#[derive(Clone, PartialEq)]
pub struct Artifact {
    pub id: String,
    pub name: String,
    pub kind: &'static str,
    pub data: PreviewData,
    /// Workspace-visible location when `data` points at an internal snapshot.
    pub location: Option<String>,
    /// Transcript item that most recently produced or mentioned this artifact.
    pub source_item: usize,
    pub superseded: bool,
    pub source_discarded: bool,
}

#[derive(Deserialize)]
pub struct FileContent {
    pub path: String,
    pub mime: String,
    pub text: Option<String>,
    pub base64: Option<String>,
    /// Set when the backend returned only a leading prefix of a large text file.
    #[serde(default)]
    pub truncated: bool,
    /// Full on-disk size (bytes), present when known.
    #[serde(default)]
    pub total_bytes: Option<u64>,
}

#[derive(Deserialize, Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    #[serde(default)]
    pub modified_unix_millis: Option<u64>,
}

#[derive(Deserialize, Clone)]
pub struct DirectoryListing {
    pub path: String,
    pub entries: Vec<DirEntry>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct UploadToContextItem {
    #[serde(default)]
    pub source_path: String,
    #[serde(default)]
    pub destination_path: String,
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub status: String,
}

#[derive(Deserialize, Clone)]
pub struct FileSearchHit {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Deserialize, Clone)]
pub struct ScratchChatInfo {
    #[serde(rename = "sessionId")]
    pub session_id: String,
}

#[derive(Deserialize, Clone)]
pub struct ProjectInfo {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub root: String,
    pub skill_count: usize,
    pub mcp_server_count: usize,
    pub memory_file_count: usize,
}

#[derive(Clone, Deserialize, PartialEq)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub workspace_dir: String,
    #[serde(default)]
    pub session_count: i64,
    #[serde(default)]
    pub artifact_count: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub running_count: i64,
    #[serde(default)]
    pub needs_you_count: i64,
    #[serde(default)]
    pub sync_configured: bool,
    #[serde(default)]
    pub last_synced_at: Option<i64>,
}

/// Read-only scan result shown before an orphaned workspace is registered and
/// its `.wisp/history` context archives are imported.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct WorkspaceSessionRecoveryPreview {
    pub workspace_dir: String,
    pub suggested_name: String,
    pub archive_count: usize,
    pub valid_archive_count: usize,
    pub recoverable_session_count: usize,
    pub message_count: usize,
    pub invalid_archive_count: usize,
    pub duplicate_archive_count: usize,
    pub earliest_message_at: Option<i64>,
    pub latest_message_at: Option<i64>,
}

/// Result of the transactional workspace-session recovery import.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct WorkspaceSessionRecoveryResult {
    pub project_id: String,
    pub project_name: String,
    pub recovered_session_count: usize,
    pub message_count: usize,
    pub invalid_archive_count: usize,
    pub duplicate_archive_count: usize,
}

/// Editable project settings (Project Settings modal). `agent_context` is the
/// project's `.wisp/WISP.md`, injected into every seeded system prompt.
#[derive(Clone, Deserialize, Default)]
pub struct ProjectSettings {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub agent_context: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SessionStatusKind {
    Running,
    NeedsYou,
    Complete,
}

impl SessionStatusKind {
    pub fn from_str(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "needs_you" => Self::NeedsYou,
            _ => Self::Complete,
        }
    }

    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::Running => "sess_status.running",
            Self::NeedsYou => "sess_status.needs_you",
            Self::Complete => "sess_status.complete",
        }
    }

    pub fn css(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::NeedsYou => "needs-you",
            Self::Complete => "complete",
        }
    }
}

/// One configured model profile (mirrors `models::ModelProfile` in src-tauri).
#[derive(Clone, Deserialize)]
pub struct ModelProfile {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub api_url: String,
    #[serde(default)]
    pub endpoint_suffix: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub has_api_key: bool,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub max_tokens: u64,
    #[serde(default = "default_model_context_window")]
    pub context_window: u64,
    #[serde(default)]
    pub reasoning_effort: String,
    #[serde(default)]
    pub service_tier: String,
    #[serde(default)]
    pub supports_vision: bool,
    #[serde(default)]
    pub use_for_vision: bool,
    #[serde(default)]
    pub use_for_image_generation: bool,
    #[serde(default)]
    pub image_size: String,
    #[serde(default)]
    pub image_quality: String,
    #[serde(default)]
    pub image_aspect_ratio: String,
    #[serde(default)]
    pub image_resolution: String,
    #[serde(default)]
    pub use_for_video_generation: bool,
    #[serde(default)]
    pub video_duration_secs: Option<u32>,
    #[serde(default)]
    pub video_aspect_ratio: Option<String>,
    #[serde(default)]
    pub video_resolution: Option<String>,
}

/// Raster image-generation model IDs. Gateway `vendor/model` ids match on the
/// last path segment. Exact IDs only — a shorter family id must not absorb a
/// longer sibling.
pub fn is_image_generation_model(model: &str) -> bool {
    let model = model.trim();
    let tail = model.rsplit('/').next().unwrap_or(model);
    tail.eq_ignore_ascii_case("gpt-image-2") || tail.eq_ignore_ascii_case("grok-imagine-image-2.0")
}

pub fn is_grok_imagine_model(model: &str) -> bool {
    let model = model.trim();
    let tail = model.rsplit('/').next().unwrap_or(model);
    tail.eq_ignore_ascii_case("grok-imagine-image-2.0")
}

pub const OPENAI_IMAGE_SIZES: &[&str] = &["auto", "1024x1024", "1536x1024", "1024x1536"];
pub const OPENAI_IMAGE_QUALITIES: &[&str] = &["auto", "low", "medium", "high"];
pub const GROK_IMAGE_ASPECT_RATIOS: &[&str] = &[
    "auto", "1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3", "2:1", "1:2", "19.5:9", "9:19.5",
    "20:9", "9:20",
];
pub const GROK_IMAGE_RESOLUTIONS: &[&str] = &["1k", "2k"];
pub const GROK_IMAGE_QUALITIES: &[&str] = &["medium", "low"];

/// Video-generation model IDs. Gateway `vendor/model` ids match on the last
/// path segment. Exact IDs only — `grok-imagine-video` must not absorb
/// `grok-imagine-video-1.5-preview` or a future sibling.
pub fn is_video_generation_model(model: &str) -> bool {
    let model = model.trim();
    let tail = model.rsplit('/').next().unwrap_or(model);
    tail.eq_ignore_ascii_case("grok-imagine-video")
        || tail.eq_ignore_ascii_case("grok-imagine-video-1.5")
        || tail.eq_ignore_ascii_case("grok-imagine-video-1.5-preview")
}

pub const VIDEO_ASPECT_RATIOS: &[&str] = &["16:9", "9:16", "1:1", "4:3", "3:4"];
pub const VIDEO_RESOLUTIONS: &[&str] = &["480p", "720p", "1080p"];
pub const VIDEO_DURATION_MIN_SECS: u32 = 1;
pub const VIDEO_DURATION_MAX_SECS: u32 = 15;
pub const VIDEO_DURATION_DEFAULT_SECS: u32 = 5;

impl ModelProfile {
    pub fn is_chat_model(&self) -> bool {
        !is_image_generation_model(&self.model) && !is_video_generation_model(&self.model)
    }
}

/// A user-definable agent persona (mirrors `specialists::Specialist` in src-tauri).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Specialist {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_backend: Option<ReviewBackendConfig>,
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    #[serde(default)]
    pub connectors: Option<Vec<String>>,
    #[serde(default)]
    pub builtin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewBackendConfig {
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
    pub fn follow_session() -> Self {
        Self::FollowSession
    }

    pub fn http(profile_id: impl Into<String>) -> Self {
        Self::HttpModel {
            profile_id: profile_id.into(),
        }
    }

    pub fn acp(profile_id: impl Into<String>) -> Self {
        Self::AcpAgent {
            profile_id: profile_id.into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewerBackendTestResult {
    pub backend: String,
    pub model: String,
    pub status: String,
    pub summary: String,
}

#[derive(Clone, Deserialize)]
pub struct RecentSession {
    pub id: String,
    pub project_id: String,
    pub title: String,
    #[serde(default)]
    pub ts: i64,
    #[serde(default)]
    pub status: String,
}

#[derive(Clone, serde::Deserialize, PartialEq)]
pub struct SkillRow {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub scope: String,
    pub enabled: bool,
    pub builtin: bool,
    #[serde(default)]
    pub managed: bool,
    #[serde(default)]
    pub managed_by: Option<String>,
    pub dir: String,
}

#[derive(Clone, serde::Deserialize, PartialEq)]
pub struct PluginRow {
    pub id: String,
    pub version: String,
    pub display_name: String,
    pub description: String,
    pub author: String,
    pub license: String,
    pub source_uri: String,
    pub archive_sha256: String,
    pub trust_state: String,
    pub enabled: bool,
    pub skill_count: usize,
    #[serde(default)]
    pub skill_names: Vec<String>,
    pub mcp_server_count: usize,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub runtime_status: String,
    #[serde(default)]
    pub runtime_errors: Vec<String>,
}

/// Named HTTP header or stdio env slot. List/persist payloads include only
/// `name` and `has_value`; `value` is write-only from the editor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct McpSecretEntry {
    pub name: String,
    pub value: Option<String>,
    pub has_value: bool,
    /// True when the client explicitly sent `has_value: false` with no new value.
    pub clear: bool,
}

impl McpSecretEntry {
    pub fn plaintext(name: impl Into<String>, value: impl Into<String>) -> Self {
        let value = value.into();
        let has_value = !value.is_empty();
        Self {
            name: name.into(),
            value: Some(value),
            has_value,
            clear: false,
        }
    }

    pub fn redacted(name: impl Into<String>, has_value: bool) -> Self {
        Self {
            name: name.into(),
            value: None,
            has_value,
            clear: false,
        }
    }
}

impl Serialize for McpSecretEntry {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("McpSecretEntry", 2)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("has_value", &self.has_value)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for McpSecretEntry {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Pair(String, String),
            Name(String),
            Obj {
                name: String,
                #[serde(default)]
                value: Option<String>,
                #[serde(default)]
                has_value: Option<bool>,
            },
        }
        match Raw::deserialize(deserializer)? {
            Raw::Pair(name, value) => Ok(Self::plaintext(name, value)),
            Raw::Name(name) => Ok(Self::redacted(name, false)),
            Raw::Obj {
                name,
                value,
                has_value,
            } => {
                let inferred = value.as_deref().is_some_and(|v| !v.trim().is_empty());
                let clear = has_value == Some(false) && !inferred;
                Ok(Self {
                    name,
                    value,
                    has_value: inferred || has_value.unwrap_or(false),
                    clear,
                })
            }
        }
    }
}

#[derive(Clone, serde::Deserialize)]
pub struct ConnRow {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub transport: ConnTransport,
}
#[derive(Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ConnTransport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: Vec<McpSecretEntry>,
        #[serde(default)]
        cwd: Option<String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: Vec<McpSecretEntry>,
        #[serde(default)]
        auth: String,
    },
}
#[derive(Clone, serde::Deserialize)]
pub struct ConnView {
    pub connections: Vec<ConnRow>,
}

// Multi-level connectors tree (bundled bio-tools domains + custom connections).
fn default_tool_mode() -> String {
    "allow".into()
}
#[derive(Clone, serde::Deserialize)]
pub struct ConnectorTool {
    pub name: String,
    #[serde(default = "default_tool_mode")]
    pub mode: String,
    #[serde(default)]
    pub description: String,
}
#[derive(Clone, serde::Deserialize)]
pub struct ConnectorInfo {
    pub key: String,
    pub name: String,
    pub kind: String,
    pub enabled: bool,
    pub skip_approvals: bool,
    pub transport: String,
    pub subtitle: String,
    #[serde(default)]
    pub auth: String,
    pub tools: Vec<ConnectorTool>,
}
#[derive(Clone, serde::Deserialize)]
pub struct ConnectorsView {
    pub connectors: Vec<ConnectorInfo>,
    /// Global approval scope: "full" | "auto" | "ask".
    pub scope: String,
}

#[derive(Clone, serde::Deserialize)]
pub struct ApprovalGrantRow {
    pub scope: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    pub kind: String,
    pub target: String,
    pub label: String,
}

/// Editor row for a header or env secret. `value` is the typed replacement;
/// empty keeps the stored secret when `has_value` is true.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ConnSecretField {
    pub name: String,
    pub value: String,
    pub has_value: bool,
}

impl ConnSecretField {
    pub fn from_entry(entry: &McpSecretEntry) -> Self {
        Self {
            name: entry.name.clone(),
            value: String::new(),
            has_value: entry.has_value
                || entry
                    .value
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty()),
        }
    }
}

// Flat form state. Secret values are write-only; listed keys arrive as has_value.
#[derive(Clone, Default)]
pub struct ConnForm {
    pub id: Option<String>,
    pub name: String,
    pub kind: String,
    pub command: String,
    pub args: String,
    pub url: String,
    pub headers: Vec<ConnSecretField>,
    pub env: Vec<ConnSecretField>,
    pub auth: String,
    pub enabled: bool,
}

impl ConnForm {
    pub fn new_connection() -> Self {
        Self {
            kind: "stdio".into(),
            enabled: true,
            headers: vec![ConnSecretField::default()],
            env: vec![ConnSecretField::default()],
            ..Self::default()
        }
    }
}

#[derive(Clone, Default)]
pub struct ModelFormEntry {
    pub row_id: u64,
    pub provider: String,
    pub endpoint_suffix: String,
    pub label: String,
    pub model: String,
    pub supports_vision: bool,
    pub use_for_vision: bool,
    pub use_for_image_generation: bool,
    pub use_for_video_generation: bool,
}

impl ModelFormEntry {
    pub fn is_image_model(&self) -> bool {
        self.use_for_image_generation || is_image_generation_model(&self.model)
    }

    pub fn is_video_model(&self) -> bool {
        self.use_for_video_generation || is_video_generation_model(&self.model)
    }
}

#[cfg(test)]
mod image_generation_model_tests {
    use super::{is_image_generation_model, ModelFormEntry, ModelProfile};

    fn profile(model: &str) -> ModelProfile {
        ModelProfile {
            id: "image".into(),
            label: "image".into(),
            provider: String::new(),
            api_url: String::new(),
            endpoint_suffix: String::new(),
            model: model.into(),
            has_api_key: false,
            active: false,
            max_tokens: 0,
            context_window: 128_000,
            reasoning_effort: String::new(),
            service_tier: String::new(),
            supports_vision: false,
            use_for_vision: false,
            use_for_image_generation: false,
            image_size: String::new(),
            image_quality: String::new(),
            image_aspect_ratio: String::new(),
            image_resolution: String::new(),
            use_for_video_generation: false,
            video_duration_secs: None,
            video_aspect_ratio: None,
            video_resolution: None,
        }
    }

    #[test]
    fn known_image_ids_are_not_chat_models() {
        for model in [
            "gpt-image-2",
            "GPT-IMAGE-2",
            "grok-imagine-image-2.0",
            "xai/grok-imagine-image-2.0",
        ] {
            assert!(is_image_generation_model(model), "{model}");
            assert!(!profile(model).is_chat_model(), "{model}");
        }
        assert!(!is_image_generation_model("grok-imagine-image"));
        assert!(!is_image_generation_model("gpt-5.5"));
        assert!(ModelFormEntry {
            model: "grok-imagine-image-2.0".into(),
            ..ModelFormEntry::default()
        }
        .is_image_model());
    }
}

#[cfg(test)]
mod video_generation_model_tests {
    use super::{is_video_generation_model, ModelFormEntry, ModelProfile};

    fn profile(model: &str) -> ModelProfile {
        ModelProfile {
            id: "video".into(),
            label: "video".into(),
            provider: String::new(),
            api_url: String::new(),
            endpoint_suffix: String::new(),
            model: model.into(),
            has_api_key: false,
            active: false,
            max_tokens: 0,
            context_window: 128_000,
            reasoning_effort: String::new(),
            service_tier: String::new(),
            supports_vision: false,
            use_for_vision: false,
            use_for_image_generation: false,
            image_size: String::new(),
            image_quality: String::new(),
            image_aspect_ratio: String::new(),
            image_resolution: String::new(),
            use_for_video_generation: false,
            video_duration_secs: None,
            video_aspect_ratio: None,
            video_resolution: None,
        }
    }

    #[test]
    fn known_video_ids_are_not_chat_models() {
        for model in [
            "grok-imagine-video",
            "Grok-Imagine-Video",
            "grok-imagine-video-1.5",
            "grok-imagine-video-1.5-preview",
            "xai/grok-imagine-video-1.5-preview",
        ] {
            assert!(is_video_generation_model(model), "{model}");
            assert!(!profile(model).is_chat_model(), "{model}");
        }
        // Exact ids only: the base id must not absorb longer siblings, and
        // image/chat models must not match.
        for model in [
            "grok-imagine-video-2.0",
            "grok-imagine-video-1.5-preview-2",
            "grok-imagine-image-2.0",
            "gpt-5.5",
        ] {
            assert!(!is_video_generation_model(model), "{model}");
        }
        assert!(profile("gpt-5.5").is_chat_model());
        assert!(ModelFormEntry {
            model: "grok-imagine-video-1.5".into(),
            ..ModelFormEntry::default()
        }
        .is_video_model());
        assert!(!ModelFormEntry {
            model: "gpt-5.5".into(),
            ..ModelFormEntry::default()
        }
        .is_video_model());
        assert!(ModelFormEntry {
            use_for_video_generation: true,
            ..ModelFormEntry::default()
        }
        .is_video_model());
    }
}

#[derive(Clone, Default)]
pub struct ModelForm {
    pub id: Option<String>,
    pub label: String,
    pub provider: String,
    pub api_url: String,
    pub endpoint_suffix: String,
    pub model: String,
    pub max_tokens: u64,
    pub context_window: u64,
    pub reasoning_effort: String,
    pub service_tier: String,
    pub supports_vision: bool,
    pub use_for_vision: bool,
    pub use_for_image_generation: bool,
    pub image_size: String,
    pub image_quality: String,
    pub image_aspect_ratio: String,
    pub image_resolution: String,
    pub use_for_video_generation: bool,
    pub video_duration_secs: Option<u32>,
    pub video_aspect_ratio: Option<String>,
    pub video_resolution: Option<String>,
    /// Used only when adding a provider (`id` is `None`): one row per model
    /// that should be created with the shared API URL and key.
    pub entries: Vec<ModelFormEntry>,
}

/// `model_catalog_lookup` projection of one baked catalog entry.
#[derive(Deserialize, Clone)]
pub struct CatalogEntryDto {
    pub context_window: u64,
    pub max_tokens: u64,
    #[serde(default)]
    #[allow(dead_code)]
    pub input_limit: Option<u64>,
    #[allow(dead_code)]
    pub supports_vision: bool,
    #[allow(dead_code)]
    pub efforts: Vec<String>,
}

fn default_model_context_window() -> u64 {
    128_000
}

#[derive(Deserialize, Clone)]
pub struct MemoryFile {
    pub name: String,
    pub bytes: u64,
}

#[derive(Deserialize, Clone)]
pub struct MemoryView {
    pub enabled: bool,
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub project_name: String,
    pub today_file: String,
    pub files: Vec<MemoryFile>,
    #[serde(default)]
    pub global_memories: Vec<GlobalMemory>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct GlobalMemory {
    pub id: String,
    pub content: String,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutoFailureAnalysisSettings {
    pub enabled: bool,
    pub failure_rate_threshold: u8,
    pub minimum_failures: u16,
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
pub struct TurnMemoryProposal {
    pub session_id: String,
    pub turn_index: usize,
    pub scope: String,
    pub content: String,
    pub trigger: String,
    pub tool_calls: usize,
    pub failed_tool_calls: usize,
    pub failure_rate: f64,
    #[serde(default)]
    pub global_memories: Vec<GlobalMemory>,
}

#[derive(Deserialize, Clone)]
pub struct BootstrapStatus {
    pub skills_loaded: usize,
    pub python_ok: bool,
    #[serde(default)]
    pub python_initializing: bool,
    pub mcp_catalog: usize,
    pub uv_ok: bool,
    pub node_ok: bool,
    pub sci_ok: bool,
    pub pixi_ok: bool,
    pub app_version: String,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub arch: String,
    pub workspace: String,
    /// Launch timings (`total=…ms … window_ready=…ms`) for bug reports.
    #[serde(default)]
    pub startup: String,
    pub errors: Vec<String>,
}

#[derive(Deserialize, Clone)]
pub struct UpdateCheck {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub release_url: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub install_supported: bool,
    #[serde(default)]
    pub downloaded: bool,
    #[serde(default)]
    pub downloading: bool,
}

#[derive(Deserialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum UpdateDownloadEvent {
    Started { content_length: Option<u64> },
    Progress { chunk_length: u64 },
    Verified,
}

#[derive(Deserialize, Clone)]
pub struct Capabilities {
    pub memory_files: Vec<MemoryFile>,
    #[serde(default)]
    pub skill_counts: CapabilitySourceCounts,
    #[serde(default)]
    pub mcp_counts: CapabilitySourceCounts,
}

#[derive(Deserialize, Clone, Copy, Default)]
pub struct CapabilitySourceCounts {
    pub bundled: usize,
    pub project: usize,
}

#[derive(Deserialize, Clone)]
pub struct OnboardingState {
    pub show: bool,
}

/// Mirrors `wisp_store::ResearchNode`. `kind` stays a plain string because the
/// backend enum serializes to snake_case and the pane only ever groups on it.
/// `metadata_json` arrives as the store's raw JSON string, not an object.
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ResearchNode {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub ref_id: Option<String>,
    pub metadata_json: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ResearchEdge {
    pub source_id: String,
    pub target_id: String,
    pub relation: String,
    pub metadata_json: String,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ResearchGraph {
    #[serde(default)]
    pub nodes: Vec<ResearchNode>,
    #[serde(default)]
    pub edges: Vec<ResearchEdge>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PublicationInfo {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub description: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PublicationRevisionInfo {
    pub id: String,
    pub publication_id: String,
    pub parent_revision_id: Option<String>,
    pub revision_number: i64,
    pub label: String,
    pub state: String,
    pub capability_level: String,
    pub manifest_sha256: Option<String>,
    pub frozen_at: Option<i64>,
    pub published_at: Option<i64>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PublicationItemInfo {
    pub id: String,
    pub revision_id: String,
    pub parent_item_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub content: String,
    pub ordinal: i64,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PublicationItemLinkInfo {
    pub source_item_id: String,
    pub target_item_id: String,
    pub relation: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PublicationEvidenceBinding {
    pub id: String,
    pub revision_id: String,
    pub item_id: Option<String>,
    pub source_kind: String,
    pub source_id: String,
    pub purpose: String,
    pub supported_claim_item_id: Option<String>,
    pub selection_state: String,
    pub review_state: String,
    pub reproduction_state: String,
    pub visibility: String,
    pub source_snapshot_json: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct PublicationEvidenceReview {
    pub binding_id: String,
    pub reviewer: String,
    pub method: String,
    pub verified_at: i64,
    pub result: String,
    pub report_json: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PublicationEvidenceSupersession {
    pub old_binding_id: String,
    pub new_binding_id: String,
    pub reason: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PublicationWaiverInfo {
    pub finding_code: String,
    pub author: String,
    pub reason: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct PublicationReadinessFinding {
    pub code: String,
    pub message: String,
    pub binding_id: Option<String>,
    pub source_id: Option<String>,
    pub waivable: bool,
    pub waived: bool,
    pub waiver: Option<PublicationWaiverInfo>,
    pub details: serde_json::Value,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct PublicationReadinessInfo {
    pub revision_id: String,
    pub target_visibility: String,
    pub capability_level: String,
    #[serde(default)]
    pub blockers: Vec<PublicationReadinessFinding>,
    #[serde(default)]
    pub warnings: Vec<PublicationReadinessFinding>,
    #[serde(default)]
    pub omissions: Vec<PublicationReadinessFinding>,
    pub manifest_sha256: String,
    pub can_freeze: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PublicationEvidenceDriftInfo {
    pub binding_id: String,
    pub bound_version_id: String,
    pub bound_version_number: i64,
    pub latest_version_id: String,
    pub latest_version_number: i64,
    pub has_drift: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PublicationLineageInfo {
    pub binding_id: String,
    pub source_label: String,
    pub quality: String,
    #[serde(default)]
    pub bases: Vec<String>,
    pub exact_version_id: Option<String>,
    pub version_number: Option<i64>,
    pub checksum: Option<String>,
    pub capture_timing: Option<String>,
    pub producing_run_id: Option<String>,
    pub run_input_count: usize,
    pub run_output_count: usize,
    pub code_snapshot_count: usize,
    pub environment_captured: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CapsuleBuildInfo {
    pub id: String,
    pub revision_id: String,
    pub format: String,
    pub visibility: String,
    pub status: String,
    pub output_path: Option<String>,
    pub revision_manifest_sha256: String,
    pub archive_sha256: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ReproductionRunInfo {
    pub id: String,
    pub source_run_id: String,
    pub status: String,
    pub capability_level: String,
    pub expected_environment_hash: Option<String>,
    pub actual_environment_hash: String,
    pub environment_matched: bool,
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
    pub exit_code: Option<i64>,
    pub error: Option<String>,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ReproductionResultInfo {
    pub reproduction_run_id: String,
    pub output_id: String,
    pub output_path: String,
    pub comparator_kind: String,
    pub required: bool,
    pub passed: bool,
    pub report_json: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct PublicationWorkspaceInfo {
    #[serde(default)]
    pub publications: Vec<PublicationInfo>,
    pub publication: Option<PublicationInfo>,
    #[serde(default)]
    pub revisions: Vec<PublicationRevisionInfo>,
    pub revision: Option<PublicationRevisionInfo>,
    #[serde(default)]
    pub items: Vec<PublicationItemInfo>,
    #[serde(default)]
    pub item_links: Vec<PublicationItemLinkInfo>,
    #[serde(default)]
    pub bindings: Vec<PublicationEvidenceBinding>,
    #[serde(default)]
    pub reviews: Vec<PublicationEvidenceReview>,
    #[serde(default)]
    pub supersessions: Vec<PublicationEvidenceSupersession>,
    #[serde(default)]
    pub waivers: Vec<PublicationWaiverInfo>,
    pub readiness: Option<PublicationReadinessInfo>,
    #[serde(default)]
    pub drift: Vec<PublicationEvidenceDriftInfo>,
    #[serde(default)]
    pub lineage: Vec<PublicationLineageInfo>,
    #[serde(default)]
    pub capsule_builds: Vec<CapsuleBuildInfo>,
    pub effective_capability_level: Option<String>,
    #[serde(default)]
    pub reproduction_runs: Vec<ReproductionRunInfo>,
    #[serde(default)]
    pub reproduction_results: Vec<ReproductionResultInfo>,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct PublicationFreezeOutcome {
    pub frozen: bool,
    pub revision: PublicationRevisionInfo,
    pub readiness: PublicationReadinessInfo,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RightTab {
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
pub struct QuickAction {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub context: String,
    pub workflow_template_id: String,
    pub enabled: bool,
    pub sort_order: i64,
    pub builtin: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WorkflowTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub proposal: DynamicAgentWorkflowProposal,
    pub builtin: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct QuickActionRun {
    pub action: QuickAction,
    pub session_id: String,
    pub display_message: String,
    pub workflow: AgentWorkflowSnapshot,
    pub started: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentWorkflowSnapshot {
    pub workflow: AgentWorkflow,
    pub delegation_enabled: bool,
    #[serde(default)]
    pub approval_policy: AgentApprovalPolicy,
    pub dynamic: DynamicAgentWorkflowSummary,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentCompletionPolicy {
    #[default]
    Inline,
    Background,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AgentCompletionSettings {
    #[serde(default)]
    pub policy: AgentCompletionPolicy,
    #[serde(default)]
    pub auto_resume: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentApprovalPolicy {
    ReviewAll,
    AutoSafe,
}

impl Default for AgentApprovalPolicy {
    fn default() -> Self {
        Self::ReviewAll
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentExecutorSelection {
    pub kind: String,
    pub profile_id: Option<String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentCapabilityOption {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub risk: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentSkillOption {
    pub id: String,
    pub name: String,
    pub scope: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentModelOption {
    pub id: String,
    pub external: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ExecutorProfileSummary {
    pub id: String,
    pub kind: String,
    pub profile_id: Option<String>,
    pub display_name: String,
    pub available: bool,
    pub supported_features: Vec<String>,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct DynamicAgentEditorOptions {
    pub capabilities: Vec<AgentCapabilityOption>,
    #[serde(default)]
    pub skills: Vec<AgentSkillOption>,
    pub models: Vec<AgentModelOption>,
    pub executors: Vec<ExecutorProfileSummary>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct AgentBudgetProposal {
    pub max_tokens: Option<u32>,
    pub max_tool_calls: Option<u32>,
    pub max_cost_microunits: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTaskKind {
    #[default]
    Agent,
    RunActivity,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RunActivityProposal {
    pub activity: String,
    pub context_id: String,
    pub input_task_id: String,
    pub spec_output_pointer: String,
    pub max_candidates: u32,
    pub max_wall_seconds: u64,
    pub max_evaluator_seconds: u64,
    pub max_cost_microunits: u64,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RunActivitySpec {
    pub activity: String,
    pub context_id: String,
    pub context_revision: String,
    pub input_task_id: String,
    pub spec_output_pointer: String,
    pub max_candidates: u32,
    pub max_wall_seconds: u64,
    pub max_evaluator_seconds: u64,
    pub max_cost_microunits: u64,
    pub provider_profile_id: Option<String>,
    pub model_profile_id: Option<String>,
    pub approval_reasons: Vec<String>,
    pub integrity_hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DynamicAgentTaskProposal {
    pub id: String,
    pub instruction: String,
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub task_kind: WorkflowTaskKind,
    #[serde(default)]
    pub run_activity: Option<RunActivityProposal>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub skill_ids: Vec<String>,
    pub specialist_id: Option<String>,
    pub output_schema: Option<serde_json::Value>,
    pub isolated: bool,
    pub model_id: Option<String>,
    pub executor: Option<AgentExecutorSelection>,
    pub budget: Option<AgentBudgetProposal>,
    /// Omit to inherit policy, use zero for unlimited, or set seconds.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DynamicAgentWorkflowProposal {
    pub goal: String,
    pub context: String,
    pub approval_policy: AgentApprovalPolicy,
    pub tasks: Vec<DynamicAgentTaskProposal>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SkillPortfolioTaskSummary {
    pub id: String,
    pub rationale: String,
    pub skill_ids: Vec<String>,
    pub depends_on: Vec<String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PortfolioPlanSummary {
    pub planner_model_id: String,
    pub planner_model_label: String,
    pub rationale: String,
    pub tasks: Vec<SkillPortfolioTaskSummary>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SkillPortfolioDraft {
    pub plan: PortfolioPlanSummary,
    pub proposal: DynamicAgentWorkflowProposal,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentExecutorSummary {
    pub kind: String,
    pub profile_id: Option<String>,
    pub model_id: Option<String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentApprovalReasonSummary {
    pub task_id: String,
    pub message: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentResultSummary {
    pub status: String,
    pub summary: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub tool_calls: i64,
    pub cost_microunits: i64,
    pub duration_secs: Option<i64>,
    pub full_result_available: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ResolvedAgentTaskSummary {
    pub id: String,
    pub stored_step_id: String,
    pub instruction: String,
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub task_kind: WorkflowTaskKind,
    #[serde(default)]
    pub run_activity: Option<RunActivitySpec>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub skill_bindings: Vec<AgentSkillBinding>,
    pub specialist_id: Option<String>,
    pub specialist_name: Option<String>,
    pub executor: AgentExecutorSummary,
    pub workspace_policy: String,
    #[serde(default)]
    pub merge_policy: String,
    pub tools: Vec<String>,
    pub can_write: bool,
    pub can_execute: bool,
    pub can_access_network: bool,
    pub budget: AgentBudgetProposal,
    pub timeout_secs: Option<u64>,
    pub approval_reasons: Vec<String>,
    pub output_schema: Option<serde_json::Value>,
    pub result: Option<AgentResultSummary>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentSkillBinding {
    pub id: String,
    pub name: String,
    pub scope: String,
    pub path: String,
    pub declared_version: Option<String>,
    pub skill_md_sha256: String,
    pub package_id: Option<String>,
    pub package_version: Option<String>,
    pub package_source: Option<String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DynamicAgentWorkflowSummary {
    pub schema_version: u32,
    pub approval_policy: AgentApprovalPolicy,
    pub editable_proposal: DynamicAgentWorkflowProposal,
    pub tasks: Vec<ResolvedAgentTaskSummary>,
    pub approval_reasons: Vec<AgentApprovalReasonSummary>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentWorkflowResultDetail {
    pub workflow_id: String,
    pub step_id: String,
    pub attempt: i64,
    pub status: String,
    pub response: serde_json::Value,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentWorkflow {
    pub id: String,
    #[serde(default)]
    pub frame_id: Option<String>,
    #[serde(default)]
    pub root_workflow_id: String,
    #[serde(default)]
    pub parent_attempt_id: Option<String>,
    #[serde(default)]
    pub depth: i64,
    pub name: String,
    pub goal: String,
    pub mode: String,
    pub status: String,
    pub max_parallel: i64,
    pub requires_confirmation: bool,
    pub version: i64,
    pub updated_at: i64,
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
pub struct ExecutionContext {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub config_json: String,
    pub capabilities_json: String,
    pub last_probe_status: Option<String>,
    pub last_probe_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct WorkspaceListing {
    pub entries: Vec<WorkspaceEntry>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct WorkspaceEntry {
    pub path: String,
    pub kind: String,
    pub size_bytes: u64,
    pub file_count: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct RemoteFileView {
    pub id: String,
    pub remote_path: String,
    pub source: String,
    pub run_id: Option<String>,
    pub run_status: Option<String>,
    pub size_bytes: Option<i64>,
    pub created_at: i64,
    pub state: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ContextDisposalReport {
    pub context_id: String,
    pub external_references: i64,
    pub staged_files: i64,
    #[serde(default)]
    pub active_runs: i64,
    #[serde(default)]
    pub sole_remote_copies: i64,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ContextStoragePrefsView {
    pub context_id: String,
    pub remote_data_root: String,
    pub remote_workdir_root: String,
    pub local_results_dir: String,
    pub confirmed: bool,
}

/// Editable state for the storage-locations dialog (first server enable in a
/// project, or the Environment rail's storage action).
#[derive(Clone, Debug, PartialEq)]
pub struct StoragePrefsForm {
    pub context_id: String,
    pub context_label: String,
    pub remote_data_root: String,
    pub remote_workdir_root: String,
    pub local_results_dir: String,
    /// True when this dialog was auto-opened on first enable.
    pub first_use: bool,
}

impl StoragePrefsForm {
    pub fn from_view(
        view: ContextStoragePrefsView,
        context_label: String,
        first_use: bool,
    ) -> Self {
        Self {
            context_id: view.context_id,
            context_label,
            remote_data_root: view.remote_data_root,
            remote_workdir_root: view.remote_workdir_root,
            local_results_dir: view.local_results_dir,
            first_use,
        }
    }
}

#[derive(Clone, Default)]
pub struct RuntimeInterpreterForm {
    pub context_id: String,
    pub context_label: String,
    pub context_kind: String,
    pub python_executable: String,
    pub rscript_executable: String,
}

impl RuntimeInterpreterForm {
    pub fn from_context(context: &ExecutionContext) -> Self {
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
pub struct TerminalSessionSummary {
    pub id: String,
    #[serde(rename = "projectId", alias = "project_id")]
    pub project_id: String,
    #[serde(rename = "contextId", alias = "context_id")]
    pub context_id: String,
    pub title: String,
    pub kind: String,
    #[serde(rename = "displayCwd", alias = "display_cwd")]
    pub display_cwd: String,
    #[serde(default, rename = "processId", alias = "process_id")]
    pub process_id: Option<u32>,
    pub running: bool,
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeKeyDto {
    pub project_id: String,
    pub context_id: String,
    pub language: String,
    #[serde(default)]
    pub scope_key: String,
    #[serde(default)]
    pub session_id: String,
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub runtime_id: String,
    pub generation: u64,
    pub key: RuntimeKeyDto,
    pub status: String,
    pub interpreter: Option<String>,
    pub version: Option<String>,
    pub process_id: Option<u32>,
    pub started_at_ms: u64,
    pub last_activity_at_ms: u64,
    pub resident_memory_bytes: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeObject {
    pub name: String,
    pub type_name: String,
    pub summary: String,
    pub size_bytes: Option<u64>,
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeObjectList {
    pub objects: Vec<RuntimeObject>,
    pub total_count: usize,
}

#[derive(Clone, Default)]
pub struct RuntimeObjectState {
    pub loading: bool,
    pub snapshot: Option<RuntimeObjectList>,
    pub error: Option<String>,
}

/// One user-driven `execute_runtime` result: console text as the agent tools
/// would render it, plus the plots the cell produced as base64-encoded PNGs.
#[derive(Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeExecutionSummary {
    pub text: String,
    #[serde(default)]
    pub plots: Vec<String>,
}

#[derive(Clone)]
pub struct RuntimeSlot {
    pub project_id: String,
    pub project_label: String,
    pub context_id: String,
    pub context_label: String,
    pub language: String,
    pub available: bool,
    pub info: Option<RuntimeInfo>,
}

/// Mirrors `wisp_store::Run`, minus the columns only the backend acts on
/// (`input_refs_json` / `output_specs_json` / `remote_handle_json` /
/// the always-NULL `script_path`). No blanket `allow(dead_code)`: an unread
/// field here means the UI is dropping data again, and the warning is the
/// whole point.
#[derive(Deserialize, Clone, PartialEq)]
pub struct RunRecord {
    pub id: String,
    pub frame_id: Option<String>,
    pub context_id: String,
    pub title: String,
    pub kind: String,
    pub status: String,
    pub command: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub exit_code: Option<i64>,
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
    #[serde(rename = "remote_workdir", alias = "remoteWorkdir")]
    pub remote_workdir: Option<String>,
    pub timeout_secs: Option<i64>,
    pub last_polled_at: Option<i64>,
    #[serde(rename = "last_poll_error", alias = "lastPollError")]
    pub last_poll_error: Option<String>,
    #[serde(default)]
    pub progress_json: String,
    pub env_snapshot_json: String,
    #[serde(default)]
    pub harvested_at: Option<i64>,
    #[serde(default)]
    pub cleaned_at: Option<i64>,
    #[serde(default)]
    pub cleanup_error: Option<String>,
}

#[derive(Deserialize, Clone, PartialEq)]
pub struct RunSummary {
    pub id: String,
    pub frame_id: Option<String>,
    pub context_id: String,
    pub title: String,
    pub kind: String,
    pub status: String,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub exit_code: Option<i64>,
    #[serde(rename = "remote_workdir", alias = "remoteWorkdir")]
    pub remote_workdir: Option<String>,
    pub timeout_secs: Option<i64>,
    pub last_polled_at: Option<i64>,
    #[serde(rename = "last_poll_error", alias = "lastPollError")]
    pub last_poll_error: Option<String>,
    #[serde(default)]
    pub progress_json: String,
    #[serde(default)]
    pub harvested_at: Option<i64>,
    #[serde(default)]
    pub cleaned_at: Option<i64>,
    #[serde(default)]
    pub cleanup_error: Option<String>,
    #[serde(default)]
    pub output_fingerprint: String,
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
            harvested_at: run.harvested_at,
            cleaned_at: run.cleaned_at,
            cleanup_error: run.cleanup_error.clone(),
            output_fingerprint: String::new(),
        }
    }
}

#[derive(Deserialize, Clone)]
pub struct MethodSearchRunState {
    pub spec_artifact_version_id: String,
    pub spec_sha256: String,
    pub control_state: String,
    pub result_status: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct MethodSearchTargetView {
    pub source_artifact_version_id: String,
    pub source_path: String,
    pub symbol: String,
}

#[derive(Deserialize, Clone)]
pub struct MethodSearchEvaluatorView {
    pub artifact_version_id: String,
    pub entry_path: String,
    pub repetitions: u32,
    pub timeout_seconds: u64,
}

#[derive(Deserialize, Clone)]
pub struct MethodSearchGuardrailView {
    pub metric: String,
    pub op: String,
    pub value: f64,
}

#[derive(Deserialize, Clone)]
pub struct MethodSearchMetricsView {
    pub primary: String,
    pub direction: String,
    pub guardrails: Vec<MethodSearchGuardrailView>,
}

#[derive(Deserialize, Clone)]
pub struct MethodSearchBudgetView {
    pub max_candidates: u32,
    pub max_wall_seconds: u64,
    pub max_evaluator_seconds: u64,
    pub max_cost_microunits: u64,
}

#[derive(Deserialize, Clone)]
pub struct MethodSearchSpecView {
    pub objective: String,
    pub target: MethodSearchTargetView,
    pub evaluator: MethodSearchEvaluatorView,
    pub metrics: MethodSearchMetricsView,
    pub protected_paths: Vec<String>,
    pub constraints: Vec<String>,
    pub budget: MethodSearchBudgetView,
    pub final_verification: Option<serde_json::Value>,
}

#[derive(Deserialize, Clone)]
pub struct BaselineAuditView {
    pub repetitions: u32,
    pub successful_repetitions: u32,
    pub failure_rate: f64,
    pub median_primary: f64,
    pub spread: f64,
    pub noise_floor: f64,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MethodSearchAuditView {
    pub baseline: BaselineAuditView,
    pub sentinel_reachable: bool,
    pub protected_files: Vec<serde_json::Value>,
    pub target_source_sha256: String,
    pub findings: Vec<String>,
}

#[derive(Deserialize, Clone)]
pub struct MethodCandidateView {
    pub id: String,
    pub parent_candidate_id: Option<String>,
    pub sequence: i64,
    pub strategy_key: String,
    pub family: String,
    pub status: String,
    pub primary_score: Option<f64>,
    pub runtime_ms: Option<i64>,
    pub changed_lines: Option<i64>,
    pub rationale: Option<String>,
    pub error: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct MethodStrategyView {
    pub strategy_key: String,
    pub category: String,
    pub weight: f64,
    pub attempts: i64,
    pub improvements: i64,
}

#[derive(Deserialize, Clone)]
pub struct MethodSearchRunOutput {
    pub artifact_version_id: String,
    pub role: String,
    pub logical_output_key: String,
    pub source_path: String,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MethodSearchRunDetails {
    pub run: RunRecord,
    pub state: MethodSearchRunState,
    pub spec: MethodSearchSpecView,
    pub audit: MethodSearchAuditView,
    pub audit_artifact_version_id: String,
    pub candidates: Vec<MethodCandidateView>,
    pub strategies: Vec<MethodStrategyView>,
    pub outputs: Vec<MethodSearchRunOutput>,
}

#[derive(Deserialize, Clone, Default)]
pub struct MethodSearchProgressView {
    #[serde(default)]
    pub phase: String,
    pub baseline_primary: Option<f64>,
    pub best_primary: Option<f64>,
    #[serde(default)]
    pub candidate_count: usize,
    #[serde(default)]
    pub successful_count: usize,
    #[serde(default)]
    pub failed_count: usize,
    #[serde(default)]
    pub cost_microunits: u64,
    pub current_strategy: Option<String>,
    pub last_checkpoint_at: Option<i64>,
    pub best_candidate_id: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct RunProgress {
    pub phase: String,
    pub direction: String,
    pub completed_bytes: u64,
    pub total_bytes: u64,
    pub files_completed: u64,
    pub files_total: u64,
    pub current_file: Option<String>,
    pub bytes_per_second: Option<u64>,
    pub eta_seconds: Option<u64>,
    pub updated_at: i64,
}

/// Provenance for a produced file — mirrors the `get_artifact_provenance`
/// Tauri command output (src-tauri `ArtifactProvenance`). Deserialize only.
#[derive(Clone, Deserialize, Default)]
pub struct ArtifactProvenance {
    pub code: String,
    pub language: String,
    pub output: String,
    #[serde(default)]
    pub inputs: Vec<ProvInput>,
    pub env: Option<ProvEnv>,
}

#[derive(Clone, Deserialize)]
pub struct ProvInput {
    pub path: String,
    pub produced_here: bool,
}

#[derive(Clone, Deserialize)]
pub struct ProvEnv {
    #[serde(default)]
    pub packages: Vec<ProvPkg>,
}

#[derive(Clone, Deserialize)]
pub struct ProvPkg {
    pub name: String,
    #[serde(default)]
    pub version: String,
}

/// Target app for `/share` social-media copy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareSocialPlatform {
    #[default]
    Xiaohongshu,
    Wechat,
    WechatMp,
    Twitter,
}

impl ShareSocialPlatform {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "xiaohongshu" => Some(Self::Xiaohongshu),
            "wechat" => Some(Self::Wechat),
            "wechat_mp" => Some(Self::WechatMp),
            "twitter" => Some(Self::Twitter),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Xiaohongshu => "xiaohongshu",
            Self::Wechat => "wechat",
            Self::WechatMp => "wechat_mp",
            Self::Twitter => "twitter",
        }
    }

    pub fn all() -> [Self; 4] {
        [
            Self::Xiaohongshu,
            Self::Wechat,
            Self::WechatMp,
            Self::Twitter,
        ]
    }

    /// Soft character budget for the caption the user pastes.
    pub fn body_limit(self) -> usize {
        match self {
            Self::Xiaohongshu => 1000,
            Self::Wechat => 500,
            Self::WechatMp => 4000,
            Self::Twitter => 280,
        }
    }

    pub fn hashtag_limit(self) -> usize {
        match self {
            Self::Xiaohongshu => 8,
            Self::Wechat => 2,
            Self::WechatMp => 10,
            Self::Twitter => 3,
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            Self::Xiaohongshu => "share.platform_xiaohongshu",
            Self::Wechat => "share.platform_wechat",
            Self::WechatMp => "share.platform_wechat_mp",
            Self::Twitter => "share.platform_twitter",
        }
    }
}

/// One highlight the model pulled from the selected transcript.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareSocialHighlight {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub why: String,
    /// 1-based indexes into the excerpt sent to the model (`[1] user`…).
    #[serde(default, alias = "messageIndexes")]
    pub message_indexes: Vec<usize>,
}

/// One paste-ready caption variant.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareSocialVariant {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub hashtags: Vec<String>,
}

/// Result of `generate_share_social_copy`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareSocialCopy {
    pub platform: ShareSocialPlatform,
    #[serde(default)]
    pub highlights: Vec<ShareSocialHighlight>,
    #[serde(default)]
    pub variants: Vec<ShareSocialVariant>,
}

/// Trajectory (轨迹) view: the whole session folded into turns of
/// user/assistant/tool/usage cells with timing and token statistics.
/// Returned by the `load_session_trajectory` command.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrajectorySnapshotDto {
    pub frame_id: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub turns: Vec<TrajectoryTurnDto>,
    #[serde(default)]
    pub stats: TrajectoryStatsDto,
}

/// One user turn (1-based index) with the cells produced while answering it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryTurnDto {
    pub index: i64,
    /// Unix epoch milliseconds of the user message that opened the turn.
    #[serde(default)]
    pub started_at: Option<i64>,
    #[serde(default)]
    pub cells: Vec<TrajectoryCellDto>,
}

/// One cell inside a trajectory turn. `kind` is one of
/// `"user" | "assistant" | "tool" | "usage"`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryCellDto {
    pub kind: String,
    /// One-line summary shown collapsed.
    #[serde(default)]
    pub summary: String,
    /// Tool cells: full arguments JSON.
    #[serde(default)]
    pub detail_input: Option<String>,
    /// Tool cells: full result text; assistant cells: full text.
    #[serde(default)]
    pub detail_output: Option<String>,
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default)]
    pub is_error: bool,
    /// Unix epoch milliseconds.
    #[serde(default)]
    pub ts: Option<i64>,
    /// Tool wall time in milliseconds.
    #[serde(default)]
    pub duration_ms: Option<i64>,
    #[serde(default)]
    pub usage: Option<TrajectoryUsageDto>,
}

/// Token accounting for one model round.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrajectoryUsageDto {
    pub round: i64,
    #[serde(default)]
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cached_input_tokens: i64,
}

/// Session-level aggregates for the trajectory header.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryStatsDto {
    pub turns: i64,
    pub steps: i64,
    pub llm_ms: i64,
    pub tool_ms: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_input_tokens: i64,
    /// cached / (input + cached) * 100; `None` when the denominator is zero.
    #[serde(default)]
    pub cache_hit_pct: Option<f64>,
    /// output_tokens / (llm_ms / 1000); `None` when llm_ms is zero.
    #[serde(default)]
    pub tokens_per_sec: Option<f64>,
}

#[cfg(test)]
mod mcp_secret_entry_tests {
    use super::McpSecretEntry;
    use serde_json::json;

    #[test]
    fn serialize_omits_secret_values() {
        let entry = McpSecretEntry::plaintext("Authorization", "secret-value");
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json, json!({"name": "Authorization", "has_value": true}));
        assert!(json.get("value").is_none());
        assert!(!json.to_string().contains("secret-value"));
    }

    #[test]
    fn deserialize_legacy_pair_keeps_value_for_migration() {
        let entry: McpSecretEntry =
            serde_json::from_value(json!(["Authorization", "secret-value"])).unwrap();
        assert_eq!(entry.name, "Authorization");
        assert_eq!(entry.value.as_deref(), Some("secret-value"));
        assert!(entry.has_value);
        assert!(!entry.clear);
    }

    #[test]
    fn deserialize_omitted_value_keeps_existing_secret() {
        let entry: McpSecretEntry =
            serde_json::from_value(json!({"name": "Authorization"})).unwrap();
        assert_eq!(entry.name, "Authorization");
        assert_eq!(entry.value, None);
        assert!(!entry.has_value);
        assert!(!entry.clear);
    }

    #[test]
    fn deserialize_explicit_has_value_false_clears() {
        let entry: McpSecretEntry =
            serde_json::from_value(json!({"name": "Authorization", "has_value": false})).unwrap();
        assert!(entry.clear);
        assert!(!entry.has_value);
    }

    #[test]
    fn deserialize_non_empty_value_sets_even_if_has_value_false() {
        let entry: McpSecretEntry = serde_json::from_value(json!({
            "name": "Authorization",
            "value": "secret-value",
            "has_value": false
        }))
        .unwrap();
        assert!(!entry.clear);
        assert_eq!(entry.value.as_deref(), Some("secret-value"));
        assert!(entry.has_value);
    }
}
