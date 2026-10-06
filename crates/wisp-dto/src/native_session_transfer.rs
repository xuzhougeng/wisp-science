//! Reviewed cross-project conversation transfer, shared by native clients.
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "wisp.native-session-transfer.v1";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Copy,
    Move,
}
impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Move => "move",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewRequest {
    pub session_id: String,
    pub target_project_id: String,
    pub mode: Mode,
}
impl PreviewRequest {
    pub fn validate(&self, project: &str) -> Result<(), String> {
        if self.session_id.trim().is_empty()
            || self.target_project_id.trim().is_empty()
            || self.target_project_id == project
            || project.starts_with("assistant:")
            || self.target_project_id.starts_with("assistant:")
        {
            return Err("Choose another research project for the conversation".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Preview {
    pub schema: String,
    pub project_id: String,
    pub session_id: String,
    pub target_project_id: String,
    pub mode: Mode,
    pub title: String,
    pub message_count: usize,
    pub revision: String,
    pub artifacts: Option<crate::SessionArtifactPreview>,
    pub artifact_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TransferRequest {
    pub session_id: String,
    pub target_project_id: String,
    pub mode: Mode,
    pub revision: String,
    pub include_artifacts: bool,
    pub artifact_fingerprint: Option<String>,
}
impl TransferRequest {
    pub fn validate(&self, project: &str) -> Result<(), String> {
        PreviewRequest {
            session_id: self.session_id.clone(),
            target_project_id: self.target_project_id.clone(),
            mode: self.mode,
        }
        .validate(project)?;
        if self.revision.len() != 64 || !self.revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("A reviewed conversation revision is required".into());
        }
        if self.include_artifacts {
            if self.mode != Mode::Move
                || self
                    .artifact_fingerprint
                    .as_deref()
                    .is_none_or(str::is_empty)
            {
                return Err("Moving artifacts requires a fresh preview and move mode".into());
            }
        } else if self.artifact_fingerprint.is_some() {
            return Err("Artifact fingerprint is only accepted when moving artifacts".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TransferResult {
    pub schema: String,
    pub project_id: String,
    pub session_id: String,
    pub target_project_id: String,
    pub mode: Mode,
    pub include_artifacts: bool,
    pub frame_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_transfer_fixture_binds_source_target_revision_and_file_choice() {
        let value: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-session-transfer/v1/move.json"
        ))
        .unwrap();
        let preview: Preview = serde_json::from_value(value["preview"].clone()).unwrap();
        let mut args: TransferRequest = serde_json::from_value(value["args"].clone()).unwrap();
        args.validate(&preview.project_id).unwrap();
        assert_eq!(args.revision, preview.revision);
        assert_eq!(
            args.artifact_fingerprint.as_deref(),
            Some(preview.artifacts.as_ref().unwrap().fingerprint.as_str())
        );
        let result: TransferResult = serde_json::from_value(value["result"].clone()).unwrap();
        assert_eq!(result.target_project_id, preview.target_project_id);
        assert_eq!(result.mode, Mode::Move);
        args.mode = Mode::Copy;
        assert!(args.validate(&preview.project_id).is_err());
        args.mode = Mode::Move;
        args.artifact_fingerprint = None;
        assert!(args.validate(&preview.project_id).is_err());
        args.include_artifacts = false;
        args.validate(&preview.project_id).unwrap();
        args.target_project_id = preview.project_id.clone();
        assert!(args.validate(&preview.project_id).is_err());
        args.target_project_id = "assistant:global".into();
        assert!(args.validate(&preview.project_id).is_err());
    }
}
