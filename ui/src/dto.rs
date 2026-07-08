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

#[derive(Deserialize, Clone)]
#[allow(dead_code)]
#[serde(tag = "kind")]
pub(crate) enum AgentEvent {
    User { frame_id: String, text: String },
    Text { frame_id: String, delta: String },
    Reasoning { frame_id: String, delta: String },
    ToolCall { frame_id: String, name: String, preview: String },
    ToolResult {
        frame_id: String,
        name: String,
        ok: bool,
        content: String,
        #[serde(default)]
        duration_ms: u64,
    },
    Usage { frame_id: String, round: u64, input: u64, output: u64, ctx_tokens: usize, max_context: usize },
    Compaction { frame_id: String, before: usize, after: usize, strategy: String },
    Diff { frame_id: String, path: String },
    Stdout { frame_id: String, chunk: String },
    Done { frame_id: String },
    Error { frame_id: String, message: String },
    Review { frame_id: String, markdown: String },
}

#[derive(Clone)]
pub(crate) enum ChatItem {
    User(String),
    QueuedUser(String),
    Assistant { text: String, model: Option<String> },
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
    ApprovalPending { tool: String, preview: String, message: String },
    Review(String),
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
            Self::Assistant { text, model } => (2u8, text, model).hash(&mut h),
            Self::Reasoning(s) => (3u8, s).hash(&mut h),
            Self::Tool { name, ok, input, output, duration_ms, .. } => {
                (4u8, name, ok, input, output, duration_ms).hash(&mut h)
            }
            Self::ApprovalPending { tool, preview, message } => (6u8, tool, preview, message).hash(&mut h),
            Self::Review(s) => (5u8, s).hash(&mut h),
        }
        h.finish()
    }
}

pub(crate) fn active_model_label(models: &[ModelProfile]) -> Option<String> {
    models.iter().find(|m| m.active).or_else(|| models.first()).map(|m| m.label.clone()).filter(|s| !s.is_empty())
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct ArtifactInfo {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) path: String,
    pub(crate) ts: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct SshHost {
    pub(crate) alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) identity_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) notes: Option<String>,
}

#[derive(Clone)]
pub(crate) enum ComposerAttachment {
    Uploading { key: String, name: String },
    Ready { key: String, name: String, path: String },
    Error { key: String, name: String, error: String },
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
    #[serde(default)]
    pub(crate) max_tokens: u64,
    #[serde(default)]
    pub(crate) reasoning_effort: String,
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
            max_tokens: 8192,
            reasoning_effort: String::new(),
        }
    }
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
    pub(crate) resume: bool,
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
    pub(crate) model_name: Option<String>,
}

impl LoadedItem {
    pub(crate) fn into_chat(self) -> ChatItem {
        match self.role.as_str() {
            "user" => ChatItem::User(self.text),
            "reasoning" => ChatItem::Reasoning(self.text),
            "tool" => ChatItem::Tool {
                name: self.tool_name.unwrap_or_else(|| "tool".into()),
                ok: self.ok,
                input: String::new(),
                output: self.text,
                started_at_ms: None,
                duration_ms: None,
            },
            _ => ChatItem::Assistant { text: self.text, model: self.model_name },
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
    Code { lang: String, body: String },
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
#[allow(dead_code)]
pub(crate) struct ProjectInfo {
    #[serde(default)] pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) root: String,
    pub(crate) skill_count: usize,
    pub(crate) mcp_server_count: usize,
    pub(crate) memory_file_count: usize,
    pub(crate) has_api_key: bool,
}

#[derive(Clone, Deserialize)]
pub(crate) struct ProjectSummary {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)] pub(crate) description: String,
    #[allow(dead_code)] #[serde(default)] pub(crate) workspace_dir: String,
    #[serde(default)] pub(crate) session_count: i64,
    #[serde(default)] pub(crate) updated_at: i64,
    #[serde(default)] pub(crate) running_count: i64,
    #[serde(default)] pub(crate) needs_you_count: i64,
}

/// Editable project settings (Project Settings modal). `agent_context` is the
/// project's `.wisp/WISP.md`, injected into every seeded system prompt.
#[derive(Clone, Deserialize, Default)]
pub(crate) struct ProjectSettings {
    #[allow(dead_code)] #[serde(default)] pub(crate) id: String,
    #[serde(default)] pub(crate) name: String,
    #[serde(default)] pub(crate) description: String,
    #[serde(default)] pub(crate) agent_context: String,
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
    #[serde(default)] pub(crate) provider: String,
    #[serde(default)] pub(crate) api_url: String,
    #[serde(default)] pub(crate) model: String,
    #[allow(dead_code)] #[serde(default)] pub(crate) has_api_key: bool,
    #[serde(default)] pub(crate) active: bool,
    #[serde(default)] pub(crate) max_tokens: u64,
    #[serde(default)] pub(crate) reasoning_effort: String,
}

#[derive(Clone, Deserialize)]
pub(crate) struct RecentSession {
    pub(crate) id: String,
    pub(crate) project_id: String,
    pub(crate) title: String,
    #[allow(dead_code)] #[serde(default)] pub(crate) ts: i64,
    #[serde(default)] pub(crate) status: String,
}

#[derive(Clone, serde::Deserialize)]
pub(crate) struct SkillRow {
    pub(crate) name: String,
    pub(crate) description: String,
    #[serde(default)] pub(crate) tags: Vec<String>,
    pub(crate) enabled: bool,
    pub(crate) builtin: bool,
    #[allow(dead_code)] pub(crate) dir: String,
}

#[derive(Clone, serde::Deserialize)]
pub(crate) struct ConnRow { pub(crate) id: String, pub(crate) name: String, pub(crate) enabled: bool, pub(crate) transport: ConnTransport }
#[derive(Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum ConnTransport {
    Stdio { command: String, #[serde(default)] args: Vec<String>, #[allow(dead_code)] #[serde(default)] env: Vec<(String,String)>, #[allow(dead_code)] #[serde(default)] cwd: Option<String> },
    Http  { url: String, #[serde(default)] headers: Vec<(String,String)> },
}
#[derive(Clone, serde::Deserialize)]
pub(crate) struct ConnView { pub(crate) connections: Vec<ConnRow> }

// Multi-level connectors tree (bundled bio-tools domains + custom connections).
#[derive(Clone, serde::Deserialize)]
pub(crate) struct ConnectorTool { pub(crate) name: String, pub(crate) mode: String }
#[derive(Clone, serde::Deserialize)]
pub(crate) struct ConnectorInfo {
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) kind: String,
    #[allow(dead_code)] pub(crate) enabled: bool,
    pub(crate) skip_approvals: bool,
    #[allow(dead_code)] pub(crate) transport: String,
    #[allow(dead_code)] pub(crate) subtitle: String,
    pub(crate) tools: Vec<ConnectorTool>,
}
#[derive(Clone, serde::Deserialize)]
pub(crate) struct ConnectorsView {
    pub(crate) connectors: Vec<ConnectorInfo>,
    /// Global approval scope: "full" | "auto" | "ask".
    pub(crate) scope: String,
}

// Simple flat form state (kind + raw text fields; args/env/headers entered as text, parsed on save).
#[derive(Clone, Default)]
pub(crate) struct ConnForm { pub(crate) id: Option<String>, pub(crate) name: String, pub(crate) kind: String, pub(crate) command: String, pub(crate) args: String, pub(crate) url: String, pub(crate) headers: String, pub(crate) enabled: bool }

#[derive(Clone, Default)]
pub(crate) struct ModelForm {
    pub(crate) id: Option<String>,
    pub(crate) label: String,
    pub(crate) provider: String,
    pub(crate) api_url: String,
    pub(crate) model: String,
    pub(crate) max_tokens: u64,
    pub(crate) reasoning_effort: String,
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
    pub(crate) mcp_catalog: usize,
    pub(crate) uv_ok: bool,
    pub(crate) node_ok: bool,
    #[allow(dead_code)] pub(crate) npm_ok: bool,
    pub(crate) sci_ok: bool,
    pub(crate) pixi_ok: bool,
    pub(crate) app_version: String,
    pub(crate) workspace: String,
    pub(crate) errors: Vec<String>,
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
pub(crate) enum RightTab { Artifacts, File, Provenance, Hosts }

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
    pub(crate) remote_workdir: Option<String>,
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
