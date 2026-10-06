//! Reviewed native exports use the same ZIP entries as the WebView exporter.
use serde::{Deserialize, Serialize};
pub const SCHEMA: &str = "wisp.native-session-export.v1";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewRequest {
    pub session_id: String,
    pub include_artifacts: bool,
}
impl PreviewRequest {
    pub fn validate(&self, project: &str) -> Result<(), String> {
        if project.trim().is_empty() || self.session_id.trim().is_empty() {
            return Err("Choose a saved conversation to export".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Artifact {
    pub path: String,
    pub mime: String,
    pub bytes: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MissingArtifact {
    pub path: String,
    pub error: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Preview {
    pub schema: String,
    pub project_id: String,
    pub session_id: String,
    pub title: String,
    pub revision: String,
    pub include_artifacts: bool,
    pub head_epoch: i64,
    pub message_count: usize,
    pub tool_call_count: usize,
    pub terminal_event_count: usize,
    pub artifacts: Vec<Artifact>,
    pub missing_artifacts: Vec<MissingArtifact>,
    pub artifact_bytes: u64,
    pub default_filename: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExportRequest {
    pub session_id: String,
    pub include_artifacts: bool,
    pub revision: String,
    pub destination_path: String,
}
impl ExportRequest {
    pub fn validate(&self, project: &str) -> Result<(), String> {
        PreviewRequest {
            session_id: self.session_id.clone(),
            include_artifacts: self.include_artifacts,
        }
        .validate(project)?;
        if self.revision.len() != 64
            || !self.revision.bytes().all(|byte| byte.is_ascii_hexdigit())
            || self.destination_path.trim().is_empty()
            || self.destination_path.contains('\0')
        {
            return Err("A reviewed export revision and destination are required".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExportResult {
    pub schema: String,
    pub project_id: String,
    pub session_id: String,
    pub revision: String,
    pub include_artifacts: bool,
    pub destination_path: String,
    pub bytes: u64,
    pub checksum: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_fixture_binds_source_file_choice_revision_and_destination() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-session-export/v1/export.json"
        ))
        .unwrap();
        let preview: Preview = serde_json::from_value(fixture["preview"].clone()).unwrap();
        let mut args: ExportRequest = serde_json::from_value(fixture["args"].clone()).unwrap();
        args.validate(&preview.project_id).unwrap();
        assert_eq!(args.revision, preview.revision);
        assert_eq!(args.include_artifacts, preview.include_artifacts);
        let result: ExportResult = serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(result.destination_path, args.destination_path);
        assert_eq!(result.revision, args.revision);
        args.revision = "unreviewed".into();
        assert!(args.validate(&preview.project_id).is_err());
        assert!(serde_json::from_value::<PreviewRequest>(serde_json::json!({"session_id":"s", "include_artifacts":false, "destination_path":"unexpected"})).is_err());
    }
}
