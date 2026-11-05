//! Runtime browsing is project-scoped and does not require creating a conversation.
use serde::{Deserialize, Serialize};

pub const COMMANDS: &[&str] = &[
    "native_context_runs_list",
    "native_context_runs_detail",
    "native_context_run_artifacts",
    "native_context_run_action",
    "native_runtimes_read",
    "native_runtimes_start",
    "native_runtimes_stop",
    "native_runtimes_restart",
    "native_runtimes_dismiss",
    "native_runtimes_inspect",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextRunsRequest {
    pub session_id: Option<String>,
    pub context_id: String,
    pub run_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextRunActionRequest {
    pub session_id: Option<String>,
    pub context_id: String,
    pub run_id: String,
    pub operation: ContextRunAction,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContextRunAction {
    Cancel {
        force: bool,
    },
    // The shortcut must never force removal of unharvested outputs.
    Cleanup {},
    Review {
        operation: crate::native_conversations::RunReviewOperation,
    },
    MethodSearch {
        operation: MethodSearchOperation,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MethodSearchOperation {
    Inspect {},
    Start {},
    Pause {},
    Resume {},
    Cancel {},
    ReadOutput { version_id: String },
    ExportOutput { version_id: String },
}

impl MethodSearchOperation {
    pub fn is_mutation(&self) -> bool {
        matches!(
            self,
            Self::Start {} | Self::Pause {} | Self::Resume {} | Self::Cancel {}
        )
    }
}

#[derive(Serialize)]
pub struct ContextRunSummary {
    #[serde(flatten)]
    pub run: crate::RunSummary,
    pub can_manage: bool,
}

/// A registered output visible from this environment. The snapshot is read by
/// artifact ID; the label is the title of the graph node, not a file path.
#[derive(Serialize, Deserialize)]
pub struct ContextRunArtifact {
    pub run_id: String,
    pub artifact_id: String,
    pub label: String,
    pub filename: String,
    pub kind: String,
    pub reader_session: String,
}

#[derive(Serialize, Deserialize)]
pub struct ContextRunsReply<T> {
    pub project_id: String,
    pub session_id: Option<String>,
    pub context_id: String,
    pub data: T,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub session_id: Option<String>,
    pub context_id: Option<String>,
    pub language: Option<String>,
    pub runtime_id: Option<String>,
    pub runtime_generation: Option<u64>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Snapshot {
    pub project_id: String,
    pub session_id: Option<String>,
    pub contexts: Vec<crate::ExecutionContext>,
    pub enabled_ids: Vec<String>,
    pub default_context_id: Option<String>,
    pub runtimes: Vec<crate::RuntimeInfo>,
    pub read_only: bool,
}

#[cfg(test)]
mod tests {
    #[test]
    fn method_search_reads_and_transitions_are_distinct() {
        use super::MethodSearchOperation as Op;
        for (name, writes) in [
            ("inspect", false),
            ("pause", true),
            ("start", true),
            ("resume", true),
            ("cancel", true),
        ] {
            let op: Op = serde_json::from_value(serde_json::json!({"kind":name})).unwrap();
            assert_eq!(op.is_mutation(), writes);
        }
        assert!(
            !serde_json::from_str::<Op>(r#"{"kind":"read_output","version_id":"v1"}"#)
                .unwrap()
                .is_mutation()
        );
        assert!(serde_json::from_str::<Op>(r#"{"kind":"export_output","path":"other"}"#).is_err());
        assert!(serde_json::from_str::<Op>(r#"{"kind":"start","force":true}"#).is_err());
    }
    #[test]
    fn environment_shortcut_cannot_force_cleanup() {
        use super::ContextRunActionRequest;
        let parse = |op: &str| {
            serde_json::from_str::<ContextRunActionRequest>(&format!(
                r#"{{"context_id":"ssh:lab","run_id":"r","operation":{op}}}"#
            ))
        };
        assert!(parse(r#"{"action":"cleanup"}"#).is_ok());
        assert!(parse(r#"{"action":"cleanup","force":true}"#).is_err());
        assert!(parse(r#"{"action":"cancel"}"#).is_err());
        assert!(parse(r#"{"action":"cancel","force":false}"#).is_ok());
    }
    #[test]
    fn environment_run_reads_accept_a_project_without_a_session() {
        use super::ContextRunsRequest;
        let list: ContextRunsRequest = serde_json::from_str(r#"{"context_id":"local"}"#).unwrap();
        assert!(list.session_id.is_none() && list.run_id.is_none());
        let detail: ContextRunsRequest = serde_json::from_str(
            r#"{"context_id":"ssh:lab","session_id":"frame-a","run_id":"run-b"}"#,
        )
        .unwrap();
        assert_eq!(detail.run_id.as_deref(), Some("run-b"));
        assert!(serde_json::from_str::<ContextRunsRequest>(r#"{"run_id":"run-b"}"#).is_err());
        assert!(serde_json::from_str::<ContextRunsRequest>(
            r#"{"context_id":"local","cancel":true}"#
        )
        .is_err());
    }

    #[test]
    fn shared_snapshot_fixture_preserves_mixed_wire_casing() {
        let value: super::Snapshot = serde_json::from_str(include_str!(
            "../../../contracts/native-runtimes/v1/snapshot.json"
        ))
        .unwrap();
        assert!(value.session_id.is_none());
        assert_eq!(value.runtimes[0].key.project_id, value.project_id);
        assert_eq!(value.runtimes[0].generation, 3);
        assert_eq!(value.contexts[0].kind, "local");
    }
    #[test]
    fn draft_request_has_no_session_or_mutation_arguments() {
        let value: super::Request = serde_json::from_str("{}").unwrap();
        assert!(value.session_id.is_none());
        assert!(value.runtime_id.is_none());
        assert!(serde_json::from_str::<super::Request>(r#"{"code":"print(1)"}"#).is_err());
    }
}
