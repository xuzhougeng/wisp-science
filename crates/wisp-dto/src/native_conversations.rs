//! Native conversation protocol, independent of either platform's UI toolkit.
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "wisp.native-conversations.v1";
pub const COMMANDS: &[&str] = &[
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
    "native_conversation_panel_contexts",
    "native_conversation_panel_context_enabled",
    "native_conversation_panel_artifacts",
    "native_conversation_panel_files",
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
    "native_conversation_snapshot",
    "native_conversation_send",
    "native_conversation_attach",
    "native_conversation_stop",
    "native_conversation_approve",
    "native_conversation_model",
];

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
    pub runtimes: Vec<crate::RuntimeInfo>,
    pub runs: Vec<crate::RunSummary>,
    pub read_only: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PanelContexts {
    pub contexts: Vec<crate::ExecutionContext>,
    pub enabled_ids: Vec<String>,
    pub read_only: bool,
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
pub struct SessionRequest {
    pub session_id: String,
    #[serde(default)]
    pub before_seq: Option<i64>,
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
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRequest {
    pub session_id: String,
    pub model_id: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Item {
    pub role: String,
    pub text: String,
    pub tool_name: Option<String>,
    pub input: Option<String>,
    pub ok: Option<bool>,
    pub status: Option<String>,
    /// Present when a saved user message carries composer files. Omitted when
    /// empty so older snapshots stay unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
}
/// A replacement event, never a delta. Sequence orders responses within one
/// host epoch. Reconnects fetch another snapshot; mutations are never replayed.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Snapshot {
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
    pub read_only: bool,
    pub model_id: String,
    pub request_id: Option<String>,
    pub error: Option<String>,
    pub approvals: Vec<super::PendingToolApproval>,
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let encoded = serde_json::to_value(snapshot).unwrap();
        assert_eq!(encoded["contexts"][0]["kind"], "local");
    }
    #[test]
    fn mutation_arguments_reject_unscoped_and_unexpected_fields() {
        assert!(
            serde_json::from_str::<SendRequest>(r#"{"message":"hello","request_id":"x"}"#).is_err()
        );
        assert!(serde_json::from_str::<ApprovalRequest>(
            r#"{"session_id":"s","approval_id":"a","approved":true,"scope":"global"}"#
        )
        .is_err());
        assert!(!COMMANDS.contains(&"send_message"));
        assert!(COMMANDS.contains(&"native_conversation_attach"));
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
