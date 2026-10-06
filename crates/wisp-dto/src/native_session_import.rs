//! Native archive import carries an explicit destination and reviewed source hash.
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "wisp.native-session-import.v1";
pub const COMMANDS: &[&str] = &[
    "native_session_archive_preview",
    "native_session_archive_import",
    "native_external_session_sources",
    "native_external_session_list",
    "native_external_session_preview",
    "native_external_session_import",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExternalSource {
    pub id: String,
    pub label: String,
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExternalSources {
    pub schema: String,
    pub project_id: String,
    pub sources: Vec<ExternalSource>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalListRequest {
    pub provider: String,
    pub context_id: String,
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExternalList {
    pub schema: String,
    pub project_id: String,
    pub provider: String,
    pub context_id: String,
    pub items: Vec<crate::ExternalSessionInfo>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalPreviewRequest {
    pub provider: String,
    pub context_id: String,
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExternalPreview {
    pub schema: String,
    pub project_id: String,
    pub provider: String,
    pub context_id: String,
    pub path: String,
    pub source_session_id: String,
    pub sha256: String,
    pub message_count: usize,
    pub messages: Vec<crate::ExternalSessionPreviewLine>,
    pub existing_session_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalImportRequest {
    pub provider: String,
    pub context_id: String,
    pub path: String,
    pub source_session_id: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExternalResult {
    pub schema: String,
    pub project_id: String,
    pub provider: String,
    pub context_id: String,
    pub path: String,
    pub source_session_id: String,
    pub frame_id: String,
    pub status: String,
    pub message_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewRequest {
    pub archive_path: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImportRequest {
    pub archive_path: String,
    pub sha256: String,
    pub source_session_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ArchivePreview {
    pub schema: String,
    pub project_id: String,
    pub archive_path: String,
    pub sha256: String,
    pub source_session_id: String,
    pub title: String,
    pub message_count: usize,
    pub artifacts: Vec<String>,
    pub messages: Vec<crate::ExternalSessionPreviewLine>,
    pub existing_session_id: Option<String>,
    pub state: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ImportResult {
    pub schema: String,
    pub project_id: String,
    pub source_session_id: String,
    pub frame_id: String,
    pub status: String,
    pub message_count: usize,
    pub artifact_count: usize,
    pub missing_artifacts: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_external_fixture_preserves_provider_source_and_destination() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-session-import/v1/external.json"
        ))
        .unwrap();
        let sources: ExternalSources = serde_json::from_value(fixture["sources"].clone()).unwrap();
        let list: ExternalList = serde_json::from_value(fixture["list"].clone()).unwrap();
        let preview: ExternalPreview = serde_json::from_value(fixture["preview"].clone()).unwrap();
        let args: ExternalImportRequest = serde_json::from_value(fixture["args"].clone()).unwrap();
        let result: ExternalResult = serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(sources.project_id, list.project_id);
        assert_eq!(list.items[0].session_id, preview.source_session_id);
        assert_eq!(preview.provider, args.provider);
        assert_eq!(preview.context_id, args.context_id);
        assert_eq!(preview.sha256, args.sha256);
        assert_eq!(result.project_id, preview.project_id);
        assert_eq!(serde_json::to_value(result).unwrap(), fixture["result"]);
        let mut bad = fixture["args"].clone();
        bad["active_window"] = "main".into();
        assert!(serde_json::from_value::<ExternalImportRequest>(bad).is_err());
        assert!(serde_json::from_value::<ExternalPreviewRequest>(
            serde_json::json!({"provider":"codex","path":"x"})
        )
        .is_err());
    }
    #[test]
    fn native_archive_fixture_roundtrips_and_requires_reviewed_identity() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-session-import/v1/archive.json"
        ))
        .unwrap();
        let preview: ArchivePreview = serde_json::from_value(fixture["preview"].clone()).unwrap();
        let request: ImportRequest = serde_json::from_value(fixture["args"].clone()).unwrap();
        let result: ImportResult = serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(preview.schema, SCHEMA);
        assert_eq!(preview.sha256, request.sha256);
        assert_eq!(preview.source_session_id, result.source_session_id);
        assert_eq!(preview.project_id, result.project_id);
        assert_eq!(serde_json::to_value(result).unwrap(), fixture["result"]);
        assert!(serde_json::from_value::<ImportRequest>(
            serde_json::json!({"archive_path":"x.zip"})
        )
        .is_err());
        let mut wrong = fixture["args"].clone();
        wrong["active_window"] = serde_json::json!("main");
        assert!(serde_json::from_value::<ImportRequest>(wrong).is_err());
    }
}
