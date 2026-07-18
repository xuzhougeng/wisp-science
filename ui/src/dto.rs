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
#[allow(dead_code)]
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
        seq: i64,
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
    Usage {
        frame_id: String,
        round: u64,
        input: u64,
        output: u64,
        ctx_tokens: usize,
        max_context: usize,
    },
    Compaction {
        frame_id: String,
        before: usize,
        after: usize,
        strategy: String,
    },
    Diff {
        frame_id: String,
        path: String,
    },
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
    /// A user turn accepted while the same session is still running.  It stays
    /// outside the active turn until the backend emits the matching User event.
    QueuedUser(String),
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
    },
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
    /// A visible handoff between the main agent and the independent reviewer.
    ReviewTransition {
        phase: ReviewTransitionPhase,
        model: Option<String>,
    },
    Review(ReviewReport),
    Plan(PlanCard),
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum ReviewTransitionPhase {
    Reviewing,
    Correcting,
    Passed,
}

impl ChatItem {
    /// Content hash used as the keyed-list key in the chat thread: a row is
    /// rebuilt only when this changes, so streaming updates to one message
    /// don't re-render the whole conversation.
    pub(crate) fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        match self {
            Self::User(s) => (0u8, s).hash(&mut h),
            Self::QueuedUser(s) => (1u8, s).hash(&mut h),
            Self::Assistant {
                text,
                model,
                resources,
            } => (2u8, text, model, resources).hash(&mut h),
            Self::Reasoning(s) => (3u8, s).hash(&mut h),
            Self::Tool {
                name,
                ok,
                input,
                output,
                duration_ms,
                ..
            } => (4u8, name, ok, input, output, duration_ms).hash(&mut h),
            Self::ApprovalPending {
                tool,
                preview,
                message,
            } => (6u8, tool, preview, message).hash(&mut h),
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
            } => (10u8, call_id, title, kind, status, content, locations).hash(&mut h),
            Self::ReviewTransition { phase, model } => (11u8, phase, model).hash(&mut h),
            Self::Review(report) => (5u8, report).hash(&mut h),
            Self::Plan(plan) => (7u8, plan).hash(&mut h),
        }
        h.finish()
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) struct PlanCard {
    pub(crate) text: String,
}

pub(crate) fn active_model_label(models: &[ModelProfile]) -> Option<String> {
    models
        .iter()
        .find(|m| m.active)
        .or_else(|| models.first())
        .map(|m| m.label.clone())
        .filter(|s| !s.is_empty())
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
    pub(crate) code: String,
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
            && self.code == code
    }

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
    Skill {
        name: String,
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
    #[serde(default)]
    pub(crate) max_tokens: u64,
    #[serde(default)]
    pub(crate) reasoning_effort: String,
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
}

fn default_sync_backend() -> String {
    "relay".into()
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
            max_tokens: 8192,
            reasoning_effort: String::new(),
            supports_vision: false,
            sync_backend: "relay".into(),
            sync_relay_url: String::new(),
            sync_folder: String::new(),
            sync_relay_token: String::new(),
            has_sync_relay_token: false,
            pet_enabled: false,
            pet_directory: String::new(),
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

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct Demo {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) request: String,
    pub(crate) response: String,
    pub(crate) thinking: Option<String>,
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

#[derive(Deserialize, Clone)]
pub(crate) struct SessionInfo {
    pub(crate) id: String,
    pub(crate) title: String,
    #[allow(dead_code)]
    pub(crate) ts: i64,
    #[serde(default)]
    pub(crate) folder_id: Option<String>,
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
}

#[derive(Deserialize)]
pub(crate) struct LoadedSessionPage {
    pub(crate) items: Vec<LoadedItem>,
    pub(crate) next_before_seq: Option<i64>,
    pub(crate) user_offset: usize,
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
#[allow(dead_code)]
pub(crate) enum PreviewData {
    Table(TableData),
    Text(String),
    Markdown(String),
    Latex { tex: String, display: bool },
    File { path: String, kind: String },
    Smiles(String),
    Fasta(String),
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
#[allow(dead_code)]
pub(crate) struct FileContent {
    pub(crate) path: String,
    pub(crate) mime: String,
    pub(crate) text: Option<String>,
    pub(crate) base64: Option<String>,
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
#[allow(dead_code)]
pub(crate) struct ProjectInfo {
    #[serde(default)]
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) root: String,
    pub(crate) skill_count: usize,
    pub(crate) mcp_server_count: usize,
    pub(crate) memory_file_count: usize,
    pub(crate) has_api_key: bool,
}

#[derive(Clone, Deserialize, PartialEq)]
pub(crate) struct ProjectSummary {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: String,
    #[allow(dead_code)]
    #[serde(default)]
    pub(crate) workspace_dir: String,
    #[serde(default)]
    pub(crate) session_count: i64,
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
    #[allow(dead_code)]
    #[serde(default)]
    pub(crate) id: String,
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
    #[allow(dead_code)]
    #[serde(default)]
    pub(crate) has_api_key: bool,
    #[serde(default)]
    pub(crate) active: bool,
    #[serde(default)]
    pub(crate) max_tokens: u64,
    #[serde(default)]
    pub(crate) reasoning_effort: String,
    #[serde(default)]
    pub(crate) supports_vision: bool,
    #[serde(default)]
    pub(crate) use_for_vision: bool,
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
    #[allow(dead_code)]
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
    pub(crate) enabled: bool,
    pub(crate) builtin: bool,
    #[allow(dead_code)]
    pub(crate) dir: String,
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
        #[allow(dead_code)]
        #[serde(default)]
        env: Vec<(String, String)>,
        #[allow(dead_code)]
        #[serde(default)]
        cwd: Option<String>,
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
    #[allow(dead_code)]
    #[serde(default, rename = "inputSchema")]
    pub(crate) input_schema: serde_json::Value,
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
    pub(crate) reasoning_effort: String,
    pub(crate) supports_vision: bool,
    pub(crate) use_for_vision: bool,
}

#[derive(Deserialize, Clone)]
#[allow(dead_code)]
pub(crate) struct MemoryFile {
    pub(crate) name: String,
    pub(crate) preview: String,
    pub(crate) bytes: u64,
}

#[derive(Deserialize, Clone)]
pub(crate) struct MemoryView {
    pub(crate) enabled: bool,
    pub(crate) today_file: String,
    pub(crate) files: Vec<MemoryFile>,
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
    #[allow(dead_code)]
    pub(crate) npm_ok: bool,
    pub(crate) sci_ok: bool,
    pub(crate) pixi_ok: bool,
    pub(crate) app_version: String,
    pub(crate) workspace: String,
    pub(crate) errors: Vec<String>,
}

#[derive(Deserialize, Clone)]
pub(crate) struct UpdateCheck {
    pub(crate) current_version: String,
    pub(crate) latest_version: String,
    pub(crate) update_available: bool,
    pub(crate) release_url: String,
}

#[derive(Deserialize, Clone)]
pub(crate) struct Capabilities {
    pub(crate) mcp_servers: Vec<String>,
    pub(crate) memory_files: Vec<MemoryFile>,
    pub(crate) project: ProjectInfo,
}

#[derive(Deserialize, Clone)]
#[allow(dead_code)]
pub(crate) struct OnboardingState {
    pub(crate) show: bool,
    pub(crate) has_api_key: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RightTab {
    Artifacts,
    Agents,
    Notebook,
    File,
    Provenance,
    Hosts,
    SideChat,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentWorkflowSnapshot {
    pub(crate) workflow: AgentWorkflow,
    pub(crate) steps: Vec<AgentWorkflowStep>,
    pub(crate) attempts: Vec<AgentWorkflowAttempt>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentWorkflow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) goal: String,
    pub(crate) mode: String,
    pub(crate) status: String,
    pub(crate) max_parallel: i64,
    pub(crate) requires_confirmation: bool,
    pub(crate) version: i64,
    pub(crate) updated_at: i64,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentWorkflowStep {
    pub(crate) id: String,
    pub(crate) position: i64,
    pub(crate) template_id: String,
    pub(crate) role: String,
    pub(crate) backend: String,
    pub(crate) model: Option<String>,
    pub(crate) permissions_json: String,
    pub(crate) budget_json: String,
    pub(crate) spec_json: String,
    pub(crate) timeout_secs: Option<i64>,
}

impl AgentWorkflowStep {
    pub(crate) fn display_name(&self) -> String {
        serde_json::from_str::<serde_json::Value>(&self.spec_json)
            .ok()
            .and_then(|value| value.get("name")?.as_str().map(str::to_string))
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| self.template_id.replace('_', " "))
    }
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentWorkflowAttempt {
    pub(crate) id: String,
    pub(crate) step_id: String,
    pub(crate) attempt: i64,
    pub(crate) status: String,
    pub(crate) output_json: String,
    pub(crate) error: Option<String>,
    pub(crate) child_frame_id: Option<String>,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) tool_calls: i64,
    pub(crate) cost_microunits: i64,
    pub(crate) cancel_requested: bool,
    pub(crate) started_at: Option<i64>,
    pub(crate) finished_at: Option<i64>,
}

#[derive(Deserialize, Clone)]
#[allow(dead_code)]
pub(crate) struct ExecutionContext {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) label: String,
    pub(crate) config_json: String,
    pub(crate) capabilities_json: String,
    pub(crate) last_probe_at: Option<i64>,
    pub(crate) last_probe_status: Option<String>,
    pub(crate) last_probe_error: Option<String>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Clone, Default)]
pub(crate) struct RuntimeInterpreterForm {
    pub(crate) context_id: String,
    pub(crate) context_label: String,
    pub(crate) python_executable: String,
    pub(crate) rscript_executable: String,
}

impl RuntimeInterpreterForm {
    pub(crate) fn from_context(context: &ExecutionContext) -> Self {
        let config =
            serde_json::from_str::<serde_json::Value>(&context.config_json).unwrap_or_default();
        let value = |keys: &[&str]| {
            keys.iter()
                .find_map(|key| config.get(*key).and_then(serde_json::Value::as_str))
                .unwrap_or_default()
                .to_string()
        };
        Self {
            context_id: context.id.clone(),
            context_label: if context.label.trim().is_empty() {
                context.id.clone()
            } else {
                context.label.clone()
            },
            python_executable: value(&["python_executable", "python_path"]),
            rscript_executable: value(&["rscript_executable", "rscript_path"]),
        }
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

#[derive(Deserialize, Clone)]
#[allow(dead_code)]
pub(crate) struct RunRecord {
    pub(crate) id: String,
    pub(crate) project_id: String,
    pub(crate) frame_id: Option<String>,
    pub(crate) context_id: String,
    pub(crate) title: String,
    pub(crate) kind: String,
    pub(crate) status: String,
    pub(crate) command: Option<String>,
    pub(crate) script_path: Option<String>,
    pub(crate) input_refs_json: String,
    pub(crate) output_specs_json: String,
    pub(crate) created_at: i64,
    pub(crate) started_at: Option<i64>,
    pub(crate) ended_at: Option<i64>,
    pub(crate) exit_code: Option<i64>,
    pub(crate) stdout_tail: Option<String>,
    pub(crate) stderr_tail: Option<String>,
    #[serde(rename = "remote_workdir", alias = "remoteWorkdir")]
    pub(crate) remote_workdir: Option<String>,
    pub(crate) remote_handle_json: Option<String>,
    pub(crate) timeout_secs: Option<i64>,
    pub(crate) last_polled_at: Option<i64>,
    #[serde(rename = "last_poll_error", alias = "lastPollError")]
    pub(crate) last_poll_error: Option<String>,
    pub(crate) env_snapshot_json: String,
}

/// Provenance for a produced file — mirrors the `get_artifact_provenance`
/// Tauri command output (src-tauri `ArtifactProvenance`). Deserialize only.
#[derive(Clone, Deserialize, Default)]
pub(crate) struct ArtifactProvenance {
    pub(crate) code: String,
    pub(crate) language: String,
    pub(crate) output: String,
    #[allow(dead_code)]
    pub(crate) exit_status: String,
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
    #[allow(dead_code)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) packages: Vec<ProvPkg>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct ProvPkg {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) version: String,
}
