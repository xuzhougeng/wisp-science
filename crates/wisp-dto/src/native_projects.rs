//! Native project commands. Payloads stay on the settings transport, but this
//! family is announced separately so a client can see project writes without
//! treating them as settings edits.
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "wisp.native-projects.v1";

pub const COMMANDS: &[&str] = &[
    "native_project_create",
    "native_project_import",
    "native_project_export",
    "native_project_import_directory",
    "native_project_recovery_preview",
    "native_project_recover_workspace",
    "native_project_folders",
    "native_project_folder_create",
    "native_project_folder_rename",
    "native_project_session_move",
];

pub fn returns_project_summary(command: &str) -> bool {
    matches!(
        command,
        "native_project_create" | "native_project_import" | "native_project_import_directory"
    )
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
pub struct SessionMoveRequest {
    pub session_id: String,
    pub folder_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ProjectFolder {
    pub id: String,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

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
