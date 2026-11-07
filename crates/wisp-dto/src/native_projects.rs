//! Native project commands. Payloads stay on the settings transport, but this
//! family is announced separately so a client can see project writes without
//! treating them as settings edits.
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "wisp.native-projects.v1";

pub const COMMANDS: &[&str] = &[
    "native_project_create",
    "native_project_import",
    "native_project_export",
    "native_project_delete",
    "native_project_import_directory",
    "native_project_recovery_preview",
    "native_project_recover_workspace",
    "native_project_folders",
    "native_project_folder_create",
    "native_project_folder_rename",
    "native_project_folder_delete",
    "native_project_folders_reorder",
    "native_project_session_move",
    "native_project_shelved_sessions",
    "native_project_session_shelve",
    "native_project_session_delete_preview",
    "native_project_home_activity",
];

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct HomeActivityRequest {
    pub known_revision: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct HomeActivityCount {
    pub project_id: String,
    pub running: i64,
    pub needs_you: i64,
}

#[derive(Serialize, Deserialize)]
pub struct HomeActivity {
    pub revision: String,
    pub privacy: crate::project_browser::PrivacyMode,
    pub projects: Vec<HomeActivityCount>,
    pub recent: Vec<crate::RecentSession>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionDeletePreviewRequest {
    pub session_ids: Vec<String>,
    pub include_children: bool,
    pub include_artifacts: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SessionDeletePreview {
    pub project_id: String,
    pub session_ids: Vec<String>,
    pub children: crate::SessionChildrenPreview,
    pub include_children: Option<bool>,
    pub artifacts: Vec<crate::SessionFamilyArtifactPreview>,
    pub artifact_error: Option<String>,
}

pub fn returns_project_summary(command: &str) -> bool {
    matches!(
        command,
        "native_project_create" | "native_project_import" | "native_project_import_directory"
    )
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SyncProjectRequest {
    pub id: String,
    pub strategy: Option<SyncConflictStrategy>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncConflictStrategy {
    Local,
    Remote,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateProjectRequest {
    pub name: String,
    pub workspace_dir: String,
    pub description: String,
    pub agent_context: String,
    pub standard_layout: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImportProjectRequest {
    pub archive_path: String,
    /// Omitted by older native clients, which import beside the archive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_parent: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Zip,
    Directory,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExportProjectRequest {
    pub destination_path: String,
    pub format: ExportFormat,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ExportProjectResult {
    pub project_id: String,
    pub destination_path: String,
    pub format: ExportFormat,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteProjectRequest {
    pub delete_data: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct DeleteProjectResult {
    pub project_id: String,
    pub delete_data: bool,
    pub deleted: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImportDirectoryRequest {
    pub directory_path: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryPreviewRequest {
    pub workspace_dir: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverWorkspaceRequest {
    pub workspace_dir: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FolderCreateRequest {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FolderRenameRequest {
    pub folder_id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FolderDeleteRequest {
    pub folder_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct FolderDeleteResult {
    pub project_id: String,
    pub folder_id: String,
    pub deleted: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FolderReorderRequest {
    pub ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct FolderReorderResult {
    pub project_id: String,
    pub ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionMoveRequest {
    pub session_id: String,
    pub folder_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ProjectFolder {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShelvedSessionsRequest {
    #[serde(default)]
    pub query: String,
    pub cursor: Option<ShelvedSessionCursor>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShelvedSessionCursor {
    pub ts: i64,
    pub id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ShelvedSession {
    pub id: String,
    pub title: String,
    pub ts: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ShelvedSessionPage {
    pub project_id: String,
    pub items: Vec<ShelvedSession>,
    pub next_cursor: Option<ShelvedSessionCursor>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionShelveRequest {
    pub session_id: String,
    pub shelved: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SessionShelveResult {
    pub project_id: String,
    pub session_id: String,
    pub shelved: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_action_fixture_keeps_explicit_scope_and_acknowledgements() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-projects/v1/folder-actions.json"
        ))
        .unwrap();
        let delete: FolderDeleteRequest =
            serde_json::from_value(fixture["delete"]["args"].clone()).unwrap();
        let deleted: FolderDeleteResult =
            serde_json::from_value(fixture["delete"]["result"].clone()).unwrap();
        assert_eq!(delete.folder_id, deleted.folder_id);
        assert_eq!(deleted.project_id, "research-1");
        assert!(deleted.deleted);
        let reorder: FolderReorderRequest =
            serde_json::from_value(fixture["reorder"]["args"].clone()).unwrap();
        let ordered: FolderReorderResult =
            serde_json::from_value(fixture["reorder"]["result"].clone()).unwrap();
        assert_eq!(reorder.ids, ordered.ids);
        assert_eq!(ordered.project_id, deleted.project_id);
        assert!(serde_json::from_value::<FolderDeleteRequest>(
            serde_json::json!({"folder_id":"f", "delete_sessions":true})
        )
        .is_err());
    }

    #[test]
    fn shelved_fixture_matches_the_explicit_project_contract() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-projects/v1/shelved.json"
        ))
        .unwrap();
        let page: ShelvedSessionPage = serde_json::from_value(fixture["page"].clone()).unwrap();
        assert_eq!(page.project_id, "research-1");
        assert_eq!(page.items[0].title, "RNA 研究");
        assert_eq!(page.next_cursor.unwrap().id, "session-a");
        let restored: SessionShelveResult =
            serde_json::from_value(fixture["restore"].clone()).unwrap();
        assert_eq!(restored.session_id, "session-a");
        assert!(!restored.shelved);
        assert!(serde_json::from_value::<SessionShelveRequest>(
            serde_json::json!({"session_id":"a"})
        )
        .is_err());
        assert!(serde_json::from_value::<ShelvedSessionsRequest>(
            serde_json::json!({"project_id":"other"})
        )
        .is_err());
    }

    #[test]
    fn home_activity_fixture_keeps_project_counts_independent_of_recent_limit() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-projects/v1/home-activity.json"
        ))
        .unwrap();
        assert!(COMMANDS.contains(&fixture["command"].as_str().unwrap()));
        let query: HomeActivityRequest = serde_json::from_value(fixture["args"].clone()).unwrap();
        assert!(query.known_revision.is_none());
        let snapshot: HomeActivity = serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(snapshot.projects[0].running, 3);
        assert_eq!(snapshot.recent.len(), 2);
        assert_eq!(serde_json::to_value(snapshot).unwrap(), fixture["result"]);
    }

    #[test]
    fn archive_import_destination_is_optional_for_older_clients() {
        let old: ImportProjectRequest = serde_json::from_value(serde_json::json!({
            "archive_path": "C:\\Exports\\study.zip"
        }))
        .unwrap();
        assert!(old.destination_parent.is_none());
        let selected: ImportProjectRequest = serde_json::from_value(serde_json::json!({
            "archive_path": "C:\\Exports\\study.zip", "destination_parent": "D:\\Research"
        }))
        .unwrap();
        assert_eq!(selected.destination_parent.as_deref(), Some("D:\\Research"));
    }

    #[test]
    fn export_contract_requires_explicit_format_and_destination() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-projects/v1/export.json"
        ))
        .unwrap();
        assert!(COMMANDS.contains(&fixture["command"].as_str().unwrap()));
        assert!(!returns_project_summary("native_project_export"));
        let request: ExportProjectRequest =
            serde_json::from_value(fixture["args"].clone()).unwrap();
        let result: ExportProjectResult =
            serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(request.format, result.format);
        assert_eq!(request.destination_path, result.destination_path);
        assert_eq!(fixture["project_id"], result.project_id);
        let mut args = fixture["args"].clone();
        args["format"] = serde_json::json!("unknown");
        assert!(serde_json::from_value::<ExportProjectRequest>(args).is_err());
        let mut args = fixture["args"].clone();
        args["active_window"] = serde_json::json!("main");
        assert!(serde_json::from_value::<ExportProjectRequest>(args).is_err());
    }

    #[test]
    fn delete_contract_requires_an_explicit_data_choice() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-projects/v1/delete.json"
        ))
        .unwrap();
        assert!(COMMANDS.contains(&fixture["command"].as_str().unwrap()));
        assert!(!returns_project_summary("native_project_delete"));
        let input: DeleteProjectRequest = serde_json::from_value(fixture["args"].clone()).unwrap();
        let result: DeleteProjectResult =
            serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(fixture["project_id"], result.project_id);
        assert_eq!(input.delete_data, result.delete_data);
        assert!(result.deleted);
        for args in [
            serde_json::json!({}),
            serde_json::json!({"delete_data": "false"}),
            serde_json::json!({"delete_data": true, "active_window": "main"}),
            serde_json::json!({"delete_data": false, "id": "other"}),
        ] {
            assert!(serde_json::from_value::<DeleteProjectRequest>(args).is_err());
        }
    }

    #[test]
    fn recovery_contract_reuses_shared_preview_and_result_shapes() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-projects/v1/recovery.json"
        ))
        .unwrap();
        for key in ["preview_request", "recover_request"] {
            assert!(COMMANDS.contains(&fixture[key]["command"].as_str().unwrap()));
            assert!(fixture[key]["project_id"].is_null());
        }
        let preview: crate::WorkspaceSessionRecoveryPreview =
            serde_json::from_value(fixture["preview_result"].clone()).unwrap();
        let result: crate::WorkspaceSessionRecoveryResult =
            serde_json::from_value(fixture["recover_result"].clone()).unwrap();
        assert_eq!(
            preview.recoverable_session_count,
            result.recovered_session_count
        );
        assert_eq!(preview.message_count, result.message_count);
        let _: RecoveryPreviewRequest =
            serde_json::from_value(fixture["preview_request"]["args"].clone()).unwrap();
        let mut args = fixture["recover_request"]["args"].clone();
        let _: RecoverWorkspaceRequest = serde_json::from_value(args.clone()).unwrap();
        args["active_window"] = serde_json::json!("main");
        assert!(serde_json::from_value::<RecoverWorkspaceRequest>(args).is_err());
    }

    #[test]
    fn directory_import_uses_an_explicit_path_without_window_scope() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-projects/v1/import-directory.json"
        ))
        .unwrap();
        let command = fixture["command"].as_str().unwrap();
        assert!(COMMANDS.contains(&command));
        assert!(returns_project_summary(command));
        assert!(fixture["project_id"].is_null());
        let input: ImportDirectoryRequest =
            serde_json::from_value(fixture["args"].clone()).unwrap();
        assert!(input.directory_path.ends_with("RNA seq"));
        let mut invalid = fixture["args"].clone();
        invalid["archive_path"] = serde_json::json!("other.zip");
        assert!(serde_json::from_value::<ImportDirectoryRequest>(invalid).is_err());
    }

    #[test]
    fn create_fixture_round_trips_without_a_project_id() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-projects/v1/create.json"
        ))
        .unwrap();
        assert_eq!(fixture["schema"], SCHEMA);
        assert!(fixture["project_id"].is_null());
        assert!(COMMANDS.contains(&fixture["command"].as_str().unwrap()));
        let request: CreateProjectRequest =
            serde_json::from_value(fixture["args"].clone()).unwrap();
        assert_eq!(request.name, "RNA-seq 研究");
        assert!(!request.standard_layout);
        assert!(request.agent_context.contains("raw data"));
        let summary: crate::ProjectSummary =
            serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(summary.id, "research-1");
        assert_eq!(summary.workspace_dir, request.workspace_dir);
        assert_eq!(summary.session_count, 0);
        let mut extra = fixture["args"].clone();
        extra["active_window"] = serde_json::json!("main");
        assert!(serde_json::from_value::<CreateProjectRequest>(extra).is_err());
        let imported: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-projects/v1/import.json"
        ))
        .unwrap();
        assert!(imported["project_id"].is_null());
        assert!(COMMANDS.contains(&imported["command"].as_str().unwrap()));
        let import_request: ImportProjectRequest =
            serde_json::from_value(imported["args"].clone()).unwrap();
        assert!(import_request.archive_path.ends_with(".zip"));
        let imported_summary: crate::ProjectSummary =
            serde_json::from_value(imported["result"].clone()).unwrap();
        assert_eq!(imported_summary.id, summary.id);
    }
}
