//! Scoped native Files browser. Existing WebView file DTOs remain the payloads.
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "wisp.native-files.v1";
pub const COMMANDS: &[&str] = &[
    "native_conversation_panel_file_locations",
    "native_conversation_panel_file_directory",
    "native_conversation_panel_file_paths",
    "native_conversation_panel_file_read",
    "native_conversation_panel_file_docx_review",
    "native_conversation_panel_file_docx_resolve",
    "native_conversation_panel_file_upload",
    "native_conversation_panel_file_download",
];
/// Project files are available before the first conversation exists.
pub const PROJECT_COMMANDS: &[&str] = &[
    "native_project_file_docx_review",
    "native_project_file_docx_resolve",
    "native_project_file_locations",
    "native_project_file_directory",
    "native_project_file_paths",
    "native_project_file_read",
    "native_project_file_search",
    "native_project_file_upload",
    "native_project_file_download",
    "native_project_file_run",
    "native_project_file_action",
    "native_project_file_save",
];

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TransferRequest {
    pub session_id: Option<String>,
    pub context_id: String,
    pub path: String,
    #[serde(default)]
    pub source_paths: Vec<String>,
    #[serde(default)]
    pub destination_path: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TransferItem {
    pub source_path: String,
    pub destination_path: Option<String>,
    pub run_id: Option<String>,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Transfer {
    pub schema: String,
    pub project_id: String,
    pub session_id: Option<String>,
    pub context_id: String,
    pub path: String,
    pub items: Vec<TransferItem>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub session_id: Option<String>,
    #[serde(default)]
    pub context_id: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub render_pdf: bool,
    #[serde(default)]
    pub render_office: bool,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub file_action: Option<crate::native_conversations::PanelFileAction>,
    #[serde(default)]
    pub new_path: Option<String>,
    #[serde(default)]
    pub original_text: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Location {
    pub context_id: String,
    pub label: String,
    pub kind: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Locations {
    pub schema: String,
    pub project_id: String,
    pub session_id: Option<String>,
    pub local_root: String,
    pub read_only: bool,
    pub locations: Vec<Location>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Directory {
    pub schema: String,
    pub project_id: String,
    pub session_id: Option<String>,
    pub context_id: String,
    pub path: String,
    pub entries: Vec<crate::DirEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PathPair {
    pub requested_path: String,
    pub relative_path: String,
    pub absolute_path: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Paths {
    pub schema: String,
    pub project_id: String,
    pub session_id: Option<String>,
    pub paths: Vec<PathPair>,
}

#[derive(Deserialize, Serialize)]
pub struct Preview {
    pub schema: String,
    pub project_id: String,
    pub session_id: Option<String>,
    pub context_id: String,
    pub requested_path: String,
    pub content: crate::FileContent,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_file_scope_is_explicitly_sessionless_and_legacy_frames_still_decode() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-files/v1/project.json"
        ))
        .unwrap();
        let locations: Locations = serde_json::from_value(fixture["locations"].clone()).unwrap();
        let directory: Directory = serde_json::from_value(fixture["directory"].clone()).unwrap();
        let paths: Paths = serde_json::from_value(fixture["paths"].clone()).unwrap();
        assert!(
            locations.session_id.is_none()
                && directory.session_id.is_none()
                && paths.session_id.is_none()
        );
        assert_eq!(
            serde_json::to_value(locations).unwrap(),
            fixture["locations"]
        );
        let project: Request = serde_json::from_str(r#"{"path":".","session_id":null}"#).unwrap();
        assert!(project.session_id.is_none());
        let frame: Request = serde_json::from_str(r#"{"session_id":"frame"}"#).unwrap();
        assert_eq!(frame.session_id.as_deref(), Some("frame"));
        assert!(PROJECT_COMMANDS
            .iter()
            .all(|name| name.starts_with("native_project_file_")));
        assert!(PROJECT_COMMANDS.iter().all(|name| !COMMANDS.contains(name)));
    }
    #[test]
    fn shared_browser_fixture_roundtrips_existing_web_file_payloads() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-files/v1/browser.json"
        ))
        .unwrap();
        let locations: Locations = serde_json::from_value(fixture["locations"].clone()).unwrap();
        assert_eq!(
            serde_json::to_value(locations).unwrap(),
            fixture["locations"]
        );
        for name in ["local_directory", "remote_directory"] {
            let directory: Directory = serde_json::from_value(fixture[name].clone()).unwrap();
            assert_eq!(serde_json::to_value(directory).unwrap(), fixture[name]);
        }
        let paths: Paths = serde_json::from_value(fixture["paths"].clone()).unwrap();
        assert_eq!(serde_json::to_value(paths).unwrap(), fixture["paths"]);
        for name in ["local_preview", "remote_preview"] {
            let preview: Preview = serde_json::from_value(fixture[name].clone()).unwrap();
            assert_eq!(serde_json::to_value(preview).unwrap(), fixture[name]);
        }
        for name in ["local_upload", "remote_upload", "remote_download"] {
            let transfer: Transfer = serde_json::from_value(fixture[name].clone()).unwrap();
            assert_eq!(serde_json::to_value(transfer).unwrap(), fixture[name]);
        }
        assert!(COMMANDS
            .iter()
            .all(|command| crate::native_conversations::COMMANDS.contains(command)));
    }

    #[test]
    fn old_entry_and_optional_request_fields_remain_compatible() {
        let entry: crate::DirEntry =
            serde_json::from_str(r#"{"name":"a.csv","is_dir":false,"size":18}"#).unwrap();
        assert_eq!(entry.modified_unix_millis, None);
        let request: Request = serde_json::from_str(r#"{"session_id":"session-1"}"#).unwrap();
        assert!(
            request.context_id.is_none()
                && request.paths.is_empty()
                && !request.render_pdf
                && !request.render_office
        );
        assert!(serde_json::from_str::<Request>(
            r#"{"session_id":"session-1","project_id":"foreign"}"#
        )
        .is_err());
    }
}
