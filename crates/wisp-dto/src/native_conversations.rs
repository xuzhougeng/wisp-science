//! Native conversation protocol, independent of either platform's UI toolkit.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const SCHEMA: &str = "wisp.native-conversations.v1";
pub const COMMANDS: &[&str] = &[
    "native_conversation_image",
    "native_conversation_queue_action",
    "native_conversation_history_action",
    "native_conversation_context",
    "native_conversation_context_undo",
    "native_conversation_references",
    "native_conversation_options",
    "native_conversation_options_set",
    "native_conversation_panel_highlight_star",
    "native_conversation_panel_side_chat",
    "native_conversation_panel_side_chat_options",
    "native_conversation_panel_notebook_stars",
    "native_conversation_panel_notebook_star",
    "native_conversation_panel_notebook_unstar",
    "native_conversation_panel_highlights",
    "native_conversation_panel_highlight_remove",
    "native_conversation_panel_agent_delegation",
    "native_conversation_panel_agent_action",
    "native_conversation_panel_agents",
    "native_conversation_panel_agent_result",
    "native_conversation_panel_runtime_start",
    "native_conversation_panel_runtime_stop",
    "native_conversation_panel_runtime_restart",
    "native_conversation_panel_runtime_dismiss",
    "native_conversation_panel_runtime_execute",
    "native_conversation_panel_activity",
    "native_conversation_panel_runtime_inspect",
    "native_conversation_panel_run_detail",
    "native_conversation_panel_run_cancel",
    "native_conversation_panel_run_harvest",
    "native_conversation_panel_run_review",
    "native_conversation_panel_contexts",
    "native_conversation_panel_context_enabled",
    "native_conversation_panel_context_default",
    "native_conversation_panel_artifacts",
    "native_conversation_panel_files",
    "native_conversation_panel_file_locations",
    "native_conversation_panel_file_directory",
    "native_conversation_panel_file_paths",
    "native_conversation_panel_file_read",
    "native_conversation_panel_file_upload",
    "native_conversation_panel_file_download",
    "native_conversation_panel_searchfiles",
    "native_conversation_panel_export",
    "native_conversation_panel_readfile",
    "native_conversation_panel_savefile",
    "native_conversation_panel_file_action",
    "native_conversation_panel_readartifact",
    "native_conversation_terminal_list",
    "native_conversation_terminal_open",
    "native_conversation_terminal_read",
    "native_conversation_terminal_write",
    "native_conversation_terminal_resize",
    "native_conversation_terminal_close",
    "native_conversation_share",
    "native_conversation_share_html",
    "native_conversation_archive_get",
    "native_conversation_archive_prepare",
    "native_conversation_archive_confirm",
    "native_conversation_archive_retry",
    "native_conversation_archive_continue",
    "native_conversation_inbox",
    "native_conversation_seen",
    "native_conversation_trajectory",
    "native_conversation_trajectory_html",
    "native_conversation_outline",
    "native_conversation_create",
    "native_conversation_rename",
    "native_conversation_transfer_preview",
    "native_conversation_transfer",
    "native_conversation_export_preview",
    "native_conversation_export",
    "native_conversation_pin",
    "native_conversation_delete",
    "native_conversation_exists",
    "native_conversation_snapshot",
    "native_conversation_send",
    "native_conversation_attach",
    "native_conversation_enqueue",
    "native_conversation_stop",
    "native_conversation_approve",
    "native_conversation_acp_permission",
    "native_conversation_acp_answer",
    "native_conversation_acp_setting",
    "native_conversation_model",
    "native_conversation_plan",
    "native_conversation_fast",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ComposerOptions {
    pub session_id: String,
    pub full_permission: bool,
    pub delegation: bool,
    pub completion: crate::AgentCompletionSettings,
    pub auto_review: bool,
    pub specialist: Option<crate::Specialist>,
    pub specialist_locked: bool,
}

/// The persisted head working set and the shared model-context breakdown.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContextView {
    pub project_id: String,
    pub session_id: String,
    pub items: Vec<Item>,
    pub details: crate::ContextUsageDetails,
    pub state: crate::SessionContextState,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextUndoRequest {
    pub session_id: String,
    pub head_epoch: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContextUndoResponse {
    pub project_id: String,
    pub session_id: String,
    /// The removed epoch, matching the shared CompactionUndone event.
    pub undone_epoch: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComposerOptionRequest {
    pub session_id: String,
    pub change: ComposerOptionChange,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComposerOptionChange {
    FullPermission {
        enabled: bool,
        confirmed: bool,
    },
    Delegation {
        enabled: bool,
    },
    Completion {
        policy: crate::AgentCompletionPolicy,
        auto_resume: bool,
    },
    AutoReview {
        enabled: bool,
    },
    Specialist {
        id: String,
    },
}

impl ComposerOptionChange {
    pub fn command(&self, session: &str) -> Result<(&'static str, serde_json::Value), String> {
        use serde_json::json;
        Ok(match self {
            Self::FullPermission {
                enabled: true,
                confirmed: false,
            } => return Err("Full permission requires explicit confirmation".into()),
            Self::FullPermission { enabled, .. } => (
                "set_session_full_permission",
                json!({"sessionId":session,"enabled":enabled}),
            ),
            Self::Delegation { enabled } => (
                "set_session_delegation_enabled",
                json!({"sessionId":session,"enabled":enabled}),
            ),
            Self::Completion {
                policy,
                auto_resume,
            } => (
                "set_session_agent_completion",
                json!({"sessionId":session,"policy":policy,"autoResume":auto_resume}),
            ),
            Self::AutoReview { enabled } => (
                "set_auto_review_enabled",
                json!({"sessionId":session,"enabled":enabled}),
            ),
            Self::Specialist { id } => {
                ("set_session_specialist", json!({"frameId":session,"id":id}))
            }
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SideChatModelOption {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentAction {
    Approve,
    Run,
    Cancel,
    Discard,
    Retry,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct PanelActivity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_review_supported: Option<bool>,
    pub runtimes: Vec<crate::RuntimeInfo>,
    pub runs: Vec<crate::RunSummary>,
    pub read_only: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PanelContexts {
    pub contexts: Vec<crate::ExecutionContext>,
    pub enabled_ids: Vec<String>,
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_context: Option<PanelDefaultContext>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PanelDefaultContext {
    /// Null inherits the global default; "local" explicitly selects this machine.
    pub context_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelFileAction {
    CreateFile,
    CreateDirectory,
    Rename,
    Delete,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PanelRequest {
    #[serde(default)]
    pub query: Option<String>,
    /// Opt in to bounded PDF page bytes instead of extracted document text.
    #[serde(default)]
    pub render_pdf: bool,
    /// Opt in to validated OOXML bytes for local Office renderers.
    #[serde(default)]
    pub render_office: bool,
    #[serde(default)]
    pub file_action: Option<PanelFileAction>,
    #[serde(default)]
    pub new_path: Option<String>,
    #[serde(default)]
    pub original_text: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub question: Option<String>,
    #[serde(default)]
    pub acp_agent_id: Option<String>,
    #[serde(default)]
    pub library_item_id: Option<String>,
    #[serde(default)]
    pub action: Option<AgentAction>,
    #[serde(default)]
    pub expected_version: Option<i64>,
    #[serde(default)]
    pub budget_overrides: Option<std::collections::HashMap<String, crate::AgentBudgetProposal>>,
    #[serde(default)]
    pub workflow_id: Option<String>,
    #[serde(default)]
    pub step_id: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub runtime_generation: Option<u64>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub runtime_id: Option<String>,
    #[serde(default)]
    pub context_id: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    pub session_id: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub artifact_id: Option<String>,
}

/// Validated local source for an explicit native Save As operation. No data is
/// copied until the user chooses a destination in the platform save dialog.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PanelExport {
    pub path: String,
    pub name: String,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TerminalInfo {
    pub id: String,
    pub project_id: String,
    pub context_id: String,
    pub title: String,
    pub kind: String,
    pub display_cwd: String,
    pub running: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TerminalOutput {
    pub terminal_id: String,
    pub start: u64,
    pub end: u64,
    pub base64: String,
    pub reset: bool,
    pub exit_code: Option<u32>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalRequest {
    pub session_id: String,
    #[serde(default)]
    pub terminal_id: Option<String>,
    #[serde(default)]
    pub context_id: Option<String>,
    #[serde(default)]
    pub cursor: Option<u64>,
    #[serde(default)]
    pub base64: Option<String>,
    #[serde(default)]
    pub rows: Option<u16>,
    #[serde(default)]
    pub cols: Option<u16>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShareRow {
    pub role: String,
    pub text: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShareExportRequest {
    pub session_id: String,
    pub rows: Vec<ShareRow>,
    #[serde(default)]
    pub dark: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveConfirmRequest {
    pub session_id: String,
    pub input: crate::ConfirmResearchArchive,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunReviewRequest {
    pub session_id: String,
    pub run_id: String,
    pub operation: RunReviewOperation,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunReviewOperation {
    CheckPrompt,
    Dismiss,
    List {
        path: String,
        name_filter: String,
        offset: usize,
    },
    Download {
        files: Vec<String>,
        dirs: Vec<String>,
    },
    Delete {
        paths: Vec<String>,
        confirmed: bool,
    },
    Cleanup {
        confirmed: bool,
    },
}
impl RunReviewOperation {
    pub fn is_mutation(&self) -> bool {
        !matches!(self, Self::List { .. } | Self::CheckPrompt)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Delete {
                confirmed: false, ..
            }
            | Self::Cleanup { confirmed: false } => {
                Err("Explicit deletion confirmation is required")
            }
            Self::Download { files, dirs }
                if files.is_empty() && dirs.is_empty() || files.len() + dirs.len() > 1000 =>
            {
                Err("Select 1–1000 files or directories")
            }
            Self::Delete { paths, .. } if paths.is_empty() || paths.len() > 1000 => {
                Err("Select 1–1000 paths")
            }
            _ => Ok(()),
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunReviewReply {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub should_prompt: Option<bool>,
    pub run_id: String,
    pub read_only: bool,
    pub cleaned: bool,
    pub listing: Option<crate::WorkspaceListing>,
    pub downloaded: Option<usize>,
    pub acknowledged: bool,
}

/// Full persisted question index. The next question's sequence is an exclusive
/// history cursor, allowing clients to locate a turn without matching its text.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OutlineEntry {
    pub user_index: usize,
    pub text: String,
    pub before_seq: Option<i64>,
    pub sent_at: Option<i64>,
    pub response_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    #[serde(default)]
    pub acp_agent_id: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionRequest {
    pub session_id: String,
    #[serde(default)]
    pub before_seq: Option<i64>,
}
/// Exactly one source: a message-bound immutable resource or a legacy project
/// path. Image reads never download a remote URL.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImageRequest {
    pub session_id: String,
    pub resource_id: Option<String>,
    pub path: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RenameRequest {
    pub session_id: String,
    pub title: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PinRequest {
    pub session_id: String,
    pub pinned: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SendRequest {
    pub session_id: String,
    pub request_id: String,
    pub message: String,
    /// Project-relative paths already copied into the workspace. Empty for a
    /// text-only send. The host folds these into the same `Uploaded files:`
    /// message the WebView persists.
    #[serde(default)]
    pub attachments: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<crate::ComposerReferenceArg>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    Artifact,
    Session,
    Skill,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceRequest {
    pub session_id: String,
    pub kind: ReferenceKind,
    pub query: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReferenceOption {
    pub reference: crate::ComposerReferenceArg,
    pub label: String,
    pub detail: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReferenceCatalog {
    pub session_id: String,
    pub options: Vec<ReferenceOption>,
}

/// Shared by native and WebView mention menus; listing a runtime never starts it.
pub fn runtime_reference_available(context: &crate::ExecutionContext, language: &str) -> bool {
    if context.kind == "local" && language == "python" {
        return true;
    }
    let config =
        serde_json::from_str::<serde_json::Value>(&context.config_json).unwrap_or_default();
    let capabilities =
        serde_json::from_str::<serde_json::Value>(&context.capabilities_json).unwrap_or_default();
    let has = |value: &serde_json::Value, key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .is_some_and(|v| !v.trim().is_empty())
    };
    match language {
        "python" => {
            ["python_executable", "python_path"]
                .iter()
                .any(|key| has(&config, key))
                || has(&capabilities, "python_executable")
        }
        "r" => {
            if ["rscript_executable", "rscript_path"]
                .iter()
                .any(|key| has(&config, key))
            {
                return true;
            }
            if has(&capabilities, "rscript_executable") {
                return capabilities.get("r_jsonlite").and_then(|v| v.as_bool()) != Some(false);
            }
            context.kind == "local" && context.last_probe_status.as_deref() != Some("ok")
        }
        _ => false,
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttachRequest {
    pub session_id: String,
    pub path: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ComposerAttachment {
    pub path: String,
    pub name: String,
}

/// Match the WebView composer: one `Uploaded files:` block, paths joined by
/// `", "`. An empty path list leaves the trimmed draft unchanged.
pub fn message_with_attachments(text: &str, paths: &[String]) -> String {
    let body = text.trim();
    if paths.is_empty() {
        return body.to_string();
    }
    let files = paths.join(", ");
    if body.is_empty() {
        format!("Uploaded files: {files}")
    } else {
        format!("{body}\n\nUploaded files: {files}")
    }
}

/// Names saved inside a user message. A reloaded snapshot shows these paths.
pub fn saved_attachment_names(text: &str) -> Vec<String> {
    text.split("\n\n")
        .find_map(|block| block.strip_prefix("Uploaded files: "))
        .map(|value| {
            value
                .split(", ")
                .filter(|path| !path.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalRequest {
    pub session_id: String,
    pub approval_id: String,
    pub approved: bool,
    #[serde(default)]
    pub feedback: Option<String>,
    #[serde(default)]
    pub scope: ApprovalScope,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalScope {
    #[default]
    Once,
    Session,
    Project,
    Global,
}
impl ApprovalScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Session => "session",
            Self::Project => "project",
            Self::Global => "global",
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcpPermissionResponse {
    pub session_id: String,
    pub request_id: String,
    pub option_id: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcpQuestionResponse {
    pub session_id: String,
    pub request_id: String,
    pub answer: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AcpPermissionOption {
    pub id: String,
    pub name: String,
    pub kind: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AcpPermission {
    pub request_id: String,
    pub frame_id: String,
    pub title: String,
    pub preview: String,
    pub options: Vec<AcpPermissionOption>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AcpInteractions {
    pub permissions: Vec<AcpPermission>,
    pub question_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcpSettingRequest {
    pub session_id: String,
    pub change: AcpSettingChange,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AcpSettingChange {
    Mode {
        id: String,
    },
    Config {
        id: String,
        value: serde_json::Value,
    },
}

impl AcpSettingChange {
    /// Validate the exact advertised IDs and value types before starting or
    /// resuming an agent. Display names must never stand in for protocol IDs.
    pub fn command(
        &self,
        state: &super::AcpSessionState,
    ) -> Result<(&'static str, serde_json::Value), String> {
        use serde_json::{json, Value};
        match self {
            Self::Mode { id } => {
                if !state
                    .modes
                    .as_ref()
                    .and_then(|m| m.get("availableModes"))
                    .and_then(Value::as_array)
                    .is_some_and(|rows| {
                        rows.iter()
                            .any(|row| row.get("id").and_then(Value::as_str) == Some(id))
                    })
                {
                    return Err("ACP mode is no longer available; refresh the conversation".into());
                }
                Ok((
                    "set_acp_session_mode",
                    json!({"frameId":state.frame_id,"modeId":id}),
                ))
            }
            Self::Config { id, value } => {
                let option = state
                    .config_options
                    .as_ref()
                    .and_then(|rows| {
                        rows.iter()
                            .find(|row| row.get("id").and_then(Value::as_str) == Some(id))
                    })
                    .ok_or("ACP configuration is no longer available; refresh the conversation")?;
                let payload = match option.get("type").and_then(Value::as_str) {
                    Some("boolean") if value.is_boolean() => {
                        json!({"type":"boolean","value":value})
                    }
                    Some("select")
                        if value.is_string()
                            && option.get("options").and_then(Value::as_array).is_some_and(
                                |rows| {
                                    rows.iter().any(|row| {
                                        row.get("value") == Some(value)
                                            || row
                                                .get("options")
                                                .and_then(Value::as_array)
                                                .is_some_and(|choices| {
                                                    choices.iter().any(|choice| {
                                                        choice.get("value") == Some(value)
                                                    })
                                                })
                                    })
                                },
                            ) =>
                    {
                        json!({"value":value})
                    }
                    _ => return Err("Unsupported ACP configuration value".into()),
                };
                Ok((
                    "set_acp_session_config",
                    json!({"frameId":state.frame_id,"configId":id,"value":payload}),
                ))
            }
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRequest {
    pub session_id: String,
    pub model_id: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRequest {
    pub session_id: String,
    pub enabled: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FastRequest {
    pub session_id: String,
    pub model_id: String,
    pub enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct FastMode {
    pub enabled: bool,
    pub inherited: bool,
}
impl FastMode {
    pub fn from_tiers(default: &str, session_override: Option<&str>) -> Self {
        Self {
            enabled: matches!(
                session_override.unwrap_or(default).trim(),
                "priority" | "fast"
            ),
            inherited: session_override.is_none(),
        }
    }
    pub fn override_for(default: &str, enabled: bool) -> Option<&'static str> {
        if Self::from_tiers(default, None).enabled == enabled {
            None
        } else if enabled {
            Some("priority")
        } else {
            Some("")
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TranscriptRun {
    pub id: String,
    pub status: String,
    /// Exact submission row in this snapshot page, never inferred by proximity.
    pub owner_index: Option<usize>,
    pub needs_review: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Item {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<crate::MessageResource>,
    /// Shared interpretation of ACP and built-in plan-mode proposals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<PlanProposal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locations: Option<String>,
    /// Accepted update_plan result; absent for failed and still-pending calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_steps: Option<Vec<crate::execution_plan::PlanStep>>,
    pub role: String,
    pub text: String,
    pub tool_name: Option<String>,
    pub input: Option<String>,
    pub ok: Option<bool>,
    pub status: Option<String>,
    /// Recorded tool elapsed time; absent in older hosts and live pending calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    /// Unix seconds from the owning persisted user turn, not a client clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
    /// Present when a saved user message carries composer files. Omitted when
    /// empty so older snapshots stay unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<TranscriptRun>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlanProposal {
    pub entries: Vec<crate::PlanEntry>,
    pub source: crate::PlanSource,
}
impl PlanProposal {
    pub fn from_text(text: &str) -> Option<Self> {
        let payload = serde_json::from_str(text).ok()?;
        let plan = crate::parse_plan_card(&payload);
        Some(Self {
            entries: plan.entries,
            source: plan.source,
        })
    }
}
/// A replacement event, never a delta. Sequence orders responses within one
/// host epoch. Reconnects fetch another snapshot; mutations are never replayed.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Snapshot {
    /// Current records for exact session-owned runs linked by this transcript.
    /// Tails are bounded; the detail command remains the full read interface.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub run_cards: Vec<crate::RunRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_review_supported: Option<bool>,
    pub schema: String,
    pub epoch: String,
    pub sequence: u64,
    pub project_id: String,
    pub session_id: String,
    pub items: Vec<Item>,
    pub next_before_seq: Option<i64>,
    #[serde(default)]
    pub user_offset: usize,
    pub running: bool,
    pub stopping: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity_status: Option<String>,
    pub read_only: bool,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composer_references: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_view: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_browser: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_transfers: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub follow_ups: Vec<String>,
    /// Absent for older hosts and ACP sessions, which own their mode selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast_mode: Option<FastMode>,
    /// Persisted ACP binding. A provisional choice before the first turn is
    /// represented by model_id = acp:<profile id>, without claiming a binding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acp_agent_id: Option<String>,
    pub request_id: Option<String>,
    pub error: Option<String>,
    pub approvals: Vec<super::PendingToolApproval>,
    /// Exact pending IDs and the scopes each request can actually grant.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub approval_scopes: HashMap<String, Vec<ApprovalScope>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acp: Option<AcpInteractions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acp_state: Option<super::AcpSessionState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_state: Option<super::native_history::HistoryState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<super::native_queue::QueueSnapshot>,
}

#[cfg(test)]
mod tests {
    #[test]
    fn context_fixture_preserves_shared_head_and_model_details() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/context.json"
        ))
        .unwrap();
        let context: super::ContextView = serde_json::from_value(fixture).unwrap();
        assert_eq!(context.project_id, "project-a");
        assert_eq!(context.session_id, "session-a");
        assert_eq!(context.items[0].role, "system");
        assert_eq!(context.state.head_epoch, 2);
        assert!(context.state.compactions[0].can_undo);
        assert_eq!(context.details.tool_definitions[0].name, "read");
        let roundtrip = serde_json::to_value(&context).unwrap();
        assert_eq!(roundtrip["state"]["head_epoch"], 2);
        assert_eq!(roundtrip["details"]["rules"], "Preserve sample IDs.");
        assert!(serde_json::from_value::<super::ContextUndoRequest>(
            serde_json::json!({"session_id":"s","head_epoch":2,"project_id":"p"})
        )
        .is_err());
    }
    #[test]
    fn native_plan_proposals_share_webview_defaults_and_preserve_markdown() {
        for source in ["native", "acp"] {
            let payload = serde_json::json!({"source":source,"entries":[
                {"content":"Inspect **samples**\n\n```python\nprint(1)\n```", "status":"in_progress", "priority":"high"},
                {"content":"Future state", "status":"blocked", "priority":"urgent"}
            ]});
            let expected = crate::parse_plan_card(&payload);
            let proposal = super::PlanProposal::from_text(&payload.to_string()).unwrap();
            assert_eq!(proposal.entries, expected.entries);
            assert_eq!(proposal.source, expected.source);
            assert_eq!(proposal.entries[1].status, crate::PlanStatus::Pending);
            assert_eq!(proposal.entries[1].priority, crate::PlanPriority::Medium);
            let wire = serde_json::to_value(&proposal).unwrap();
            assert_eq!(wire["source"], source);
            assert_eq!(wire["entries"][0]["status"], "in_progress");
            assert_eq!(
                wire["entries"][0]["content"],
                payload["entries"][0]["content"]
            );
        }
        assert!(super::PlanProposal::from_text("not JSON").is_none());
        for text in ["null", "{}", r#"{"entries":"invalid"}"#] {
            assert!(super::PlanProposal::from_text(text)
                .unwrap()
                .entries
                .is_empty());
        }
    }

    #[test]
    fn acp_settings_validate_protocol_ids_types_and_grouped_choices() {
        use super::AcpSettingChange;
        use serde_json::json;
        let state = crate::AcpSessionState {
            frame_id: "owner".into(),
            modes: Some(
                json!({"currentModeId":"read", "availableModes":[{"id":"agent","name":"Agent"}]}),
            ),
            config_options: Some(vec![
                json!({"id":"depth","type":"select","options":[{"value":"short"},{"name":"Group","options":[{"value":"deep","name":"Deep"}]}]}),
                json!({"id":"confirm","type":"boolean"}),
                json!({"id":"future","type":"unknown"}),
            ]),
        };
        let mode = AcpSettingChange::Mode { id: "agent".into() }
            .command(&state)
            .unwrap();
        assert_eq!(mode.0, "set_acp_session_mode");
        assert_eq!(mode.1, json!({"frameId":"owner","modeId":"agent"}));
        assert!(AcpSettingChange::Mode { id: "Agent".into() }
            .command(&state)
            .is_err());
        let select = AcpSettingChange::Config {
            id: "depth".into(),
            value: json!("deep"),
        }
        .command(&state)
        .unwrap();
        assert_eq!(
            select.1,
            json!({"frameId":"owner","configId":"depth","value":{"value":"deep"}})
        );
        let boolean = AcpSettingChange::Config {
            id: "confirm".into(),
            value: json!(false),
        }
        .command(&state)
        .unwrap();
        assert_eq!(boolean.1["value"], json!({"type":"boolean","value":false}));
        for (id, value) in [
            ("depth", json!("Deep")),
            ("confirm", json!("false")),
            ("missing", json!(true)),
            ("future", json!("x")),
        ] {
            assert!(AcpSettingChange::Config {
                id: id.into(),
                value
            }
            .command(&state)
            .is_err());
        }
    }
    #[test]
    fn composer_references_preserve_typed_ids_and_old_send_compatibility() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/composer-references.json"
        ))
        .unwrap();
        let request: super::ReferenceRequest =
            serde_json::from_value(fixture["request"].clone()).unwrap();
        assert_eq!(request.kind, super::ReferenceKind::Artifact);
        let catalog: super::ReferenceCatalog =
            serde_json::from_value(fixture["response"].clone()).unwrap();
        assert_eq!(catalog.session_id, "session-a");
        assert_eq!(catalog.options.len(), 7);
        assert_eq!(serde_json::to_value(&catalog).unwrap(), fixture["response"]);
        let send: super::SendRequest = serde_json::from_value(fixture["send"].clone()).unwrap();
        assert_eq!(
            send.references,
            vec![
                crate::ComposerReferenceArg::Artifact {
                    id: "artifact-a".into()
                },
                crate::ComposerReferenceArg::Skill {
                    name: "RNA-seq".into()
                }
            ]
        );
        let old: super::SendRequest = serde_json::from_value(fixture["old_send"].clone()).unwrap();
        assert!(old.references.is_empty() && old.attachments.is_empty());
        let mut invalid = fixture["request"].clone();
        invalid["kind"] = "shell".into();
        assert!(serde_json::from_value::<super::ReferenceRequest>(invalid).is_err());
    }

    #[test]
    fn runtime_references_follow_shared_capability_rules() {
        let mut context = crate::ExecutionContext {
            id: "local".into(),
            kind: "local".into(),
            label: "Local".into(),
            config_json: "{}".into(),
            capabilities_json: "{}".into(),
            last_probe_status: None,
            last_probe_error: None,
        };
        assert!(super::runtime_reference_available(&context, "python"));
        assert!(super::runtime_reference_available(&context, "r"));
        context.last_probe_status = Some("ok".into());
        assert!(!super::runtime_reference_available(&context, "r"));
        context.kind = "ssh".into();
        assert!(!super::runtime_reference_available(&context, "python"));
        context.capabilities_json =
            r#"{"rscript_executable":"/bin/Rscript","r_jsonlite":false}"#.into();
        assert!(!super::runtime_reference_available(&context, "r"));
        context.config_json =
            r#"{"rscript_path":"/custom/Rscript","python_path":"/env/python"}"#.into();
        assert!(super::runtime_reference_available(&context, "r"));
        assert!(super::runtime_reference_available(&context, "python"));
        assert!(!super::runtime_reference_available(&context, "bash"));
    }

    #[test]
    fn composer_options_are_scoped_and_permission_requires_confirmation() {
        let options: super::ComposerOptions = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/composer-options.json"
        ))
        .unwrap();
        assert_eq!(options.session_id, "session-a");
        assert!(!options.full_permission && !options.delegation && !options.specialist_locked);
        assert_eq!(
            options.completion.policy,
            crate::AgentCompletionPolicy::Inline
        );
        let denied = super::ComposerOptionChange::FullPermission {
            enabled: true,
            confirmed: false,
        };
        assert!(denied.command("session-a").is_err());
        let allowed = super::ComposerOptionChange::FullPermission {
            enabled: true,
            confirmed: true,
        };
        let (command, args) = allowed.command("session-a").unwrap();
        assert_eq!(command, "set_session_full_permission");
        assert_eq!(
            args,
            serde_json::json!({"sessionId":"session-a", "enabled":true})
        );
        let (_, args) = super::ComposerOptionChange::AutoReview { enabled: false }
            .command("session-b")
            .unwrap();
        assert_eq!(args["sessionId"], "session-b");
        for bad in [
            serde_json::json!({"change":{"kind":"auto_review","enabled":true}}),
            serde_json::json!({"session_id":"s","change":{"kind":"full_permission","enabled":true}}),
            serde_json::json!({"session_id":"s","change":{"kind":"arbitrary_command","enabled":true}}),
            serde_json::json!({"session_id":"s","change":{"kind":"auto_review","enabled":true,"sessionId":"other"}}),
        ] {
            assert!(serde_json::from_value::<super::ComposerOptionRequest>(bad).is_err());
        }
    }

    #[test]
    fn composer_helpers_reuse_global_preferences_and_full_specialist_contracts() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/composer-helpers.json"
        ))
        .unwrap();
        let memory: crate::MemoryView = serde_json::from_value(fixture["memory"].clone()).unwrap();
        assert_eq!(memory.project_id, "project-a");
        assert!(memory.enabled);
        let analysis: crate::AutoFailureAnalysisSettings =
            serde_json::from_value(fixture["analysis"].clone()).unwrap();
        assert_eq!(
            analysis,
            crate::AutoFailureAnalysisSettings {
                enabled: true,
                ..Default::default()
            }
        );
        let specialists: Vec<crate::Specialist> =
            serde_json::from_value(fixture["specialists"].clone()).unwrap();
        assert_eq!(specialists[0].id, "reviewer");
        assert_eq!(
            specialists[0].skills.as_deref(),
            Some(["synthetic-skill".into()].as_slice())
        );
        assert_eq!(
            specialists[0].review_backend,
            Some(crate::ReviewBackendConfig::http("chat-a"))
        );
        let models: Vec<crate::ModelProfile> =
            serde_json::from_value(fixture["models"].clone()).unwrap();
        assert_eq!(
            models
                .iter()
                .filter(|model| model.is_chat_model())
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["chat-a", "sibling"]
        );
        let agents: Vec<crate::AcpAgentProfile> =
            serde_json::from_value(fixture["agents"].clone()).unwrap();
        assert_eq!(agents[0].id, "agent-a");
    }

    #[test]
    fn run_review_contract_requires_explicit_destructive_confirmation() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/run-review.json"
        ))
        .unwrap();
        let request: RunReviewRequest = serde_json::from_value(fixture["request"].clone()).unwrap();
        assert!(!request.operation.is_mutation());
        assert!(request.operation.validate().is_ok());
        let reply: RunReviewReply = serde_json::from_value(fixture["reply"].clone()).unwrap();
        assert_eq!(reply.run_id, "run-a");
        assert_eq!(reply.listing.unwrap().entries[1].kind, "dir");
        for operation in [
            RunReviewOperation::Delete {
                paths: vec!["a".into()],
                confirmed: false,
            },
            RunReviewOperation::Cleanup { confirmed: false },
            RunReviewOperation::Download {
                files: vec![],
                dirs: vec![],
            },
        ] {
            assert!(operation.is_mutation());
            assert!(operation.validate().is_err());
        }
        assert!(RunReviewOperation::Cleanup { confirmed: true }
            .validate()
            .is_ok());
        assert!(serde_json::from_value::<RunReviewOperation>(
            serde_json::json!({"action":"cleanup"})
        )
        .is_err());
        assert!(serde_json::from_value::<RunReviewOperation>(
            serde_json::json!({"action":"execute","command":"x"})
        )
        .is_err());
        assert!(serde_json::from_value::<RunReviewOperation>(
            serde_json::json!({"action":"cleanup","confirmed":true,"path":"/"})
        )
        .is_err());
    }
    #[test]
    fn transcript_runs_are_optional_and_preserve_page_ownership() {
        let items: Vec<Item> = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/transcript-runs.json"
        ))
        .unwrap();
        assert!(items[0].run.is_none());
        let run = items[3].run.as_ref().unwrap();
        assert_eq!(run.id, "run-a");
        assert_eq!(run.owner_index, Some(1));
        assert_eq!(run.status, "succeeded");
        assert!(!run.needs_review);
        let mut old = serde_json::to_value(&items[3]).unwrap();
        old.as_object_mut().unwrap().remove("run");
        assert!(serde_json::from_value::<Item>(old).unwrap().run.is_none());
    }
    use super::*;

    #[test]
    fn image_queue_fixture_retains_immutable_bindings_and_full_width_ids() {
        let snapshot: Snapshot = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/snapshot-images-queue.json"
        ))
        .unwrap();
        assert_eq!(
            snapshot.items[1].resources[0]
                .artifact_version_id
                .as_deref(),
            Some("version-a")
        );
        let value = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(
            value["items"][1]["resources"][0]["originalReference"],
            "figures/qc.png"
        );
        assert_eq!(value["queue"]["items"][0]["id"], "18446744073709551615");
        assert_eq!(value["queue"]["items"][1]["id"], "9007199254740993");
        assert!(COMMANDS.contains(&"native_conversation_image"));
        assert!(
            serde_json::from_str::<ImageRequest>(r#"{"session_id":"s","path":"p.png"}"#).is_ok()
        );
        assert!(serde_json::from_str::<ImageRequest>(
            r#"{"session_id":"s","resource_id":"r","project_id":"other"}"#
        )
        .is_err());
        assert!(serde_json::from_str::<ImageRequest>(r#"{"path":"p.png"}"#).is_err());
    }
    #[test]
    fn fast_preserves_default_and_explicit_off_semantics() {
        for default in ["", "default", "priority", "fast"] {
            for enabled in [true, false] {
                let value = FastMode::override_for(default, enabled);
                let state = FastMode::from_tiers(default, value);
                assert_eq!(state.enabled, enabled);
                assert_eq!(
                    state.inherited,
                    FastMode::from_tiers(default, None).enabled == enabled
                );
            }
        }
        assert!(!FastMode::from_tiers("priority", Some("")).enabled);
        assert_eq!(FastMode::override_for("priority", false), Some(""));
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/fast.json"
        ))
        .unwrap();
        assert!(COMMANDS.contains(&fixture["command"].as_str().unwrap()));
        let request: FastRequest = serde_json::from_value(fixture["args"].clone()).unwrap();
        assert_eq!(request.model_id, "model-a");
        assert!(request.enabled);
        assert!(
            serde_json::from_str::<FastRequest>(r#"{"session_id":"s","enabled":true}"#).is_err()
        );
        assert!(serde_json::from_str::<FastRequest>(
            r#"{"session_id":"s","model_id":"m","enabled":"true"}"#
        )
        .is_err());
    }
    #[test]
    fn plan_mode_is_optional_and_requests_are_explicit() {
        let mut snapshot: Snapshot = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/snapshot.json"
        ))
        .unwrap();
        assert_eq!(snapshot.plan_mode, None);
        assert_eq!(snapshot.fast_mode, None);
        snapshot.plan_mode = Some(false);
        assert_eq!(serde_json::to_value(&snapshot).unwrap()["plan_mode"], false);
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/plan.json"
        ))
        .unwrap();
        assert!(COMMANDS.contains(&fixture["command"].as_str().unwrap()));
        let request: PlanRequest = serde_json::from_value(fixture["args"].clone()).unwrap();
        assert!(request.enabled);
        for invalid in [
            r#"{"session_id":"s"}"#,
            r#"{"session_id":"s","enabled":"true"}"#,
            r#"{"session_id":"s","enabled":true,"project_id":"other"}"#,
        ] {
            assert!(serde_json::from_str::<PlanRequest>(invalid).is_err());
        }
    }
    #[test]
    fn pin_request_requires_explicit_boolean_state() {
        let request: PinRequest =
            serde_json::from_str(r#"{"session_id":"s","pinned":true}"#).unwrap();
        assert!(request.pinned);
        assert_eq!(request.session_id, "s");
        for invalid in [
            r#"{"session_id":"s"}"#,
            r#"{"session_id":"s","pinned":"false"}"#,
            r#"{"session_id":"s","pinned":false,"project_id":"other"}"#,
        ] {
            assert!(serde_json::from_str::<PinRequest>(invalid).is_err());
        }
    }
    #[test]
    fn rename_requires_an_explicit_session_and_title() {
        let request: RenameRequest =
            serde_json::from_str(r#"{"session_id":"s","title":"样本分析"}"#).unwrap();
        assert_eq!(request.session_id, "s");
        assert_eq!(request.title, "样本分析");
        assert!(serde_json::from_str::<RenameRequest>(r#"{"title":"new"}"#).is_err());
        assert!(serde_json::from_str::<RenameRequest>(
            r#"{"session_id":"s","title":"new","project_id":"other"}"#
        )
        .is_err());
    }
    #[test]
    fn create_preserves_http_default_and_accepts_explicit_acp_profile() {
        assert!(serde_json::from_str::<CreateRequest>("{}")
            .unwrap()
            .acp_agent_id
            .is_none());
        assert_eq!(
            serde_json::from_str::<CreateRequest>(r#"{"acp_agent_id":"agent"}"#)
                .unwrap()
                .acp_agent_id
                .as_deref(),
            Some("agent")
        );
        assert!(serde_json::from_str::<CreateRequest>(r#"{"command":"run arbitrary"}"#).is_err());
    }
    #[test]
    fn acp_interactions_are_scoped_and_optional_for_older_snapshots() {
        let mut value: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/acp-interactions.json"
        ))
        .unwrap();
        let snapshot: Snapshot = serde_json::from_value(value.clone()).unwrap();
        let pending = snapshot.acp.unwrap();
        assert_eq!(pending.question_ids, ["ask-1"]);
        assert_eq!(pending.permissions[0].frame_id, snapshot.session_id);
        assert_eq!(pending.permissions[0].options[1].kind, "allow_always");
        value.as_object_mut().unwrap().remove("acp");
        assert!(serde_json::from_value::<Snapshot>(value)
            .unwrap()
            .acp
            .is_none());
        assert!(
            serde_json::from_value::<AcpQuestionResponse>(serde_json::json!({
                "session_id":"s", "request_id":"a", "answer":"yes", "project_id":"other"
            }))
            .is_err()
        );
    }
    #[test]
    fn panel_fixtures_use_existing_file_contracts() {
        let files: Vec<crate::DirEntry> = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/panel-files.json"
        ))
        .unwrap();
        assert!(files[0].is_dir);
        assert_eq!(files[1].name, "README.md");
        let preview: crate::FileContent = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/panel-preview.json"
        ))
        .unwrap();
        assert!(preview.truncated);
        assert_eq!(preview.total_bytes, Some(8_000_000));
    }
    #[test]
    fn terminal_fixture_has_explicit_raw_byte_cursor() {
        let output: TerminalOutput = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/terminal-output.json"
        ))
        .unwrap();
        assert_eq!(output.start, 0);
        assert_eq!(output.end, 5);
        assert!(output.reset);
        assert_eq!(output.terminal_id, "terminal-a");
    }
    #[test]
    fn share_fixture_excludes_tool_machinery() {
        let rows: Vec<ShareRow> = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/share.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].role, "reasoning");
        assert!(rows.iter().all(|row| row.role != "tool"));
    }
    #[test]
    fn archive_fixture_uses_existing_review_contract() {
        let archive: crate::ResearchArchive = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/archive.json"
        ))
        .unwrap();
        assert_eq!(archive.project_id, "project-a");
        assert_eq!(archive.files[0].action, "snapshot");
        assert!(!archive.files[0].can_delete);
        assert!(archive.frozen_at.is_none());
        assert!(serde_json::from_str::<ArchiveConfirmRequest>(r#"{"input":{}}"#).is_err());
    }
    #[test]
    fn inbox_fixture_preserves_cross_project_navigation_identity() {
        let rows: Vec<crate::SessionSearchInfo> = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/inbox.json"
        ))
        .unwrap();
        assert_eq!(rows[0].project_id, "project-a");
        assert_eq!(rows[1].project_id, "project-b");
        assert_eq!(rows[1].id, "session-b");
        assert!(rows.iter().all(|row| row.status == "needs_you"));
    }
    #[test]
    fn trajectory_fixture_uses_existing_shared_contract() {
        let snapshot: crate::TrajectorySnapshotDto = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/trajectory.json"
        ))
        .unwrap();
        assert_eq!(snapshot.frame_id, "session-a");
        assert_eq!(snapshot.turns[0].cells[0].duration_ms, Some(40));
        assert!(snapshot.turns[0].cells[0].is_error);
        assert_eq!(snapshot.stats.output_tokens, 20);
    }
    #[test]
    fn outline_fixture_retains_indexes_and_exclusive_cursors() {
        let rows: Vec<OutlineEntry> = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/outline.json"
        ))
        .unwrap();
        assert_eq!(rows[0].text, rows[1].text);
        assert_eq!(rows[0].before_seq, Some(8));
        assert_eq!(rows[1].before_seq, None);
        assert_eq!(rows[1].user_index, 1);
    }
    #[test]
    fn shared_native_conversation_fixtures_roundtrip() {
        let snapshot: Snapshot = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/snapshot.json"
        ))
        .unwrap();
        assert_eq!(snapshot.schema, SCHEMA);
        assert_eq!(snapshot.items[1].text, "正在检查样本…");
        assert_eq!(snapshot.approvals[0].frame_id, snapshot.session_id);
        let send: SendRequest = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/send.json"
        ))
        .unwrap();
        assert_eq!(
            snapshot.request_id.as_deref(),
            Some(send.request_id.as_str())
        );
        let approval: ApprovalRequest = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/approval.json"
        ))
        .unwrap();
        assert!(!approval.approved);
        assert_eq!(approval.approval_id, snapshot.approvals[0].approval_id);
        let encoded = serde_json::to_value(snapshot).unwrap();
        assert_eq!(encoded["items"][0]["tool_name"], serde_json::Value::Null);
    }
    #[test]
    fn side_chat_fixture_preserves_evidence_and_session_identity() {
        let response: crate::SideChatResponse = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/panel-side-chat.json"
        ))
        .unwrap();
        assert_eq!(response.session_id.as_deref(), Some("session-a"));
        assert_eq!(response.snapshot_version, 42);
        assert_eq!(response.evidence[0].event_seq, Some(40));
        assert_eq!(response.evidence[0].message_seq, None);
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["sessionId"], "session-a");
        assert_eq!(value["noEvidence"], false);
        let options: Vec<SideChatModelOption> = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/panel-side-chat-options.json"
        ))
        .unwrap();
        assert_eq!(options[1].kind, "acp");
    }
    #[test]
    fn highlights_reuse_library_item_contract() {
        let rows: Vec<crate::LibraryItem> = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/panel-highlights.json"
        ))
        .unwrap();
        assert_eq!(rows[0].kind, "text");
        assert_eq!(rows[0].source_session_id, "session-a");
        assert_eq!(rows[0].code.as_ref(), "样本 质量\n合格");
        let value = serde_json::to_value(rows).unwrap();
        assert_eq!(value[0]["source_project_id"], "project-a");
    }
    #[test]
    fn provenance_fixture_reuses_recorded_transcript_items() {
        let rows: Vec<Item> = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/panel-provenance.json"
        ))
        .unwrap();
        let tools: Vec<_> = rows.iter().filter(|row| row.role == "tool").collect();
        assert_eq!(tools.len(), 4);
        assert_eq!(tools[0].ok, Some(true));
        assert_eq!(tools[1].ok, Some(false));
        assert_eq!(tools[2].ok, None);
        assert_eq!(tools[0].text, "42\n");
        assert_eq!(tools[1].input.as_deref(), Some("样本.csv"));
    }
    #[test]
    fn delegation_read_and_disabled_write_remain_distinct() {
        let read: PanelRequest =
            serde_json::from_value(serde_json::json!({"session_id":"s"})).unwrap();
        assert!(
            !read.render_pdf && !read.render_office,
            "legacy clients retain extracted document text"
        );
        let preview: PanelRequest = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/panel-pdf-preview.json"
        ))
        .unwrap();
        assert!(preview.render_pdf);
        assert!(preview.render_office);
        let pdf_only: PanelRequest =
            serde_json::from_value(serde_json::json!({"session_id":"s", "render_pdf":true}))
                .unwrap();
        assert!(!pdf_only.render_office);
        assert_eq!(preview.path.as_deref(), Some("literature/paper.pdf"));
        let write: PanelRequest =
            serde_json::from_value(serde_json::json!({"session_id":"s", "enabled":false})).unwrap();
        assert_eq!(read.enabled, None);
        assert_eq!(write.enabled, Some(false));
    }
    #[test]
    fn agent_actions_are_closed_and_approval_keeps_reviewed_version() {
        let args: PanelRequest = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/panel-agent-action.json"
        ))
        .unwrap();
        assert_eq!(args.action, Some(AgentAction::Approve));
        assert_eq!(args.expected_version, Some(7));
        assert!(serde_json::from_value::<PanelRequest>(
            serde_json::json!({"session_id":"s", "action":"delete_project"})
        )
        .is_err());
        let args: PanelRequest = serde_json::from_value(serde_json::json!({"session_id":"s", "action":"retry", "budget_overrides":{"review":{"max_tokens":0}}})).unwrap();
        assert_eq!(args.budget_overrides.unwrap()["review"].max_tokens, Some(0));
    }
    #[test]
    fn agent_panel_fixtures_preserve_workflow_and_step_identity() {
        let rows: Vec<crate::AgentWorkflowSnapshot> = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/panel-agents.json"
        ))
        .unwrap();
        assert_eq!(rows[0].workflow.frame_id.as_deref(), Some("session-a"));
        assert_eq!(rows[0].dynamic.tasks[0].stored_step_id, "workflow-a:review");
        let result: crate::AgentWorkflowResultDetail = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/panel-agent-result.json"
        ))
        .unwrap();
        assert_eq!(result.step_id, rows[0].dynamic.tasks[0].stored_step_id);
        assert_eq!(result.attempt, 1);
    }
    #[test]
    fn panel_activity_fixtures_use_existing_runtime_and_run_shapes() {
        let activity: PanelActivity = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/panel-activity.json"
        ))
        .unwrap();
        assert_eq!(activity.runtimes[0].key.session_id, "session-a");
        assert_eq!(activity.runtimes[0].generation, 2);
        assert_eq!(activity.runs[0].status, "running");
        let run: crate::RunRecord = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/panel-run.json"
        ))
        .unwrap();
        assert_eq!(run.stdout_tail.as_deref(), Some("Processed 10 samples"));
        let objects: crate::RuntimeObjectList = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/panel-runtime-objects.json"
        ))
        .unwrap();
        assert_eq!(objects.objects[0].name, "samples");
        assert_eq!(objects.total_count, 1);
        let execution: crate::RuntimeExecutionSummary = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/panel-runtime-execution.json"
        ))
        .unwrap();
        assert_eq!(execution.text, "[stdout]\n42");
        assert!(execution.plots.is_empty());
    }
    #[test]
    fn panel_context_fixture_preserves_session_membership() {
        let snapshot: PanelContexts = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/panel-contexts.json"
        ))
        .unwrap();
        assert_eq!(snapshot.contexts.len(), 3);
        assert_eq!(snapshot.enabled_ids, vec!["ssh:gpu"]);
        assert!(!snapshot.read_only);
        assert!(snapshot.default_context.is_none());
        let encoded = serde_json::to_value(snapshot).unwrap();
        assert_eq!(encoded["contexts"][0]["kind"], "local");
    }
    #[test]
    fn panel_default_context_preserves_inheritance_and_explicit_selection() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/context-default.json"
        ))
        .unwrap();
        assert!(COMMANDS.contains(&fixture["command"].as_str().unwrap()));
        let request: PanelRequest = serde_json::from_value(fixture["args"].clone()).unwrap();
        assert_eq!(request.context_id.as_deref(), Some("remote-a"));
        for id in [None, Some("local"), Some("remote-a")] {
            let encoded = serde_json::to_value(PanelDefaultContext {
                context_id: id.map(str::to_owned),
            })
            .unwrap();
            let decoded: PanelDefaultContext = serde_json::from_value(encoded).unwrap();
            assert_eq!(decoded.context_id.as_deref(), id);
        }
    }
    #[test]
    fn mutation_arguments_reject_unscoped_and_unexpected_fields() {
        assert!(
            serde_json::from_str::<SendRequest>(r#"{"message":"hello","request_id":"x"}"#).is_err()
        );
        assert!(serde_json::from_str::<ApprovalRequest>(
            r#"{"session_id":"s","approval_id":"a","approved":true,"scope":"everything"}"#
        )
        .is_err());
        assert!(!COMMANDS.contains(&"send_message"));
        assert!(COMMANDS.contains(&"native_conversation_attach"));
    }
    #[test]
    fn native_approval_scope_defaults_once_and_accepts_only_the_shared_names() {
        for scope in ["once", "session", "project", "global"] {
            let request: ApprovalRequest = serde_json::from_value(serde_json::json!({"session_id":"s","approval_id":"a","approved":true,"scope":scope})).unwrap();
            assert_eq!(request.scope.as_str(), scope);
        }
        let old: ApprovalRequest = serde_json::from_value(
            serde_json::json!({"session_id":"s","approval_id":"a","approved":true}),
        )
        .unwrap();
        assert_eq!(old.scope, ApprovalScope::Once);
    }
    #[test]
    fn saved_snapshot_keeps_composer_attachments() {
        let item: Item = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/attached-item.json"
        ))
        .unwrap();
        assert_eq!(item.role, "user");
        assert_eq!(saved_attachment_names(&item.text), item.attachments);
        assert_eq!(
            message_with_attachments("看看", &item.attachments),
            item.text
        );
        let attach: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/attach.json"
        ))
        .unwrap();
        assert_eq!(attach["command"], "native_conversation_attach");
        assert_eq!(attach["project_id"], "project-a");
        let request: AttachRequest = serde_json::from_value(attach["args"].clone()).unwrap();
        assert_eq!(request.session_id, "session-a");
        assert!(!request.path.is_empty());
        let saved: ComposerAttachment = serde_json::from_value(attach["result"].clone()).unwrap();
        assert_eq!(saved.path, "uploads/notes.csv");
        let send: SendRequest = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/send.json"
        ))
        .unwrap();
        assert!(send.attachments.is_empty());
    }
}
