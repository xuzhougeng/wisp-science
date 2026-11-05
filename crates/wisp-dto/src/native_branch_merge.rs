//! Native branch actions reuse the WebView's checkpoint and guard-hash contract.
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewRequest {
    pub session_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateRequest {
    pub session_id: String,
    pub expected_guard_hash: String,
    pub current_version: Option<String>,
    pub user_guidance: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MergeRequest {
    pub session_id: String,
    pub expected_guard_hash: String,
    pub summary: String,
}

impl GenerateRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.session_id.trim().is_empty() || self.expected_guard_hash.trim().is_empty() {
            return Err("Branch and preview guard are required");
        }
        match (&self.current_version, &self.user_guidance) {
            (None, None) => Ok(()),
            (Some(draft), Some(guidance))
                if !draft.trim().is_empty() && !guidance.trim().is_empty()
                    && draft.chars().count() <= 64_000 && guidance.chars().count() <= 8_000 => Ok(()),
            _ => Err("Guided generation requires a current draft and nonempty guidance within the size limits"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_branch_payloads_and_guidance_boundaries() {
        let data: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-conversations/v1/branch-merge.json"
        ))
        .unwrap();
        let preview: crate::SessionBranchMergePreview =
            serde_json::from_value(data["preview"]["result"].clone()).unwrap();
        let merged: crate::SessionBranchMerge =
            serde_json::from_value(data["merged"]["result"].clone()).unwrap();
        assert_eq!(preview.branch_session_id, merged.branch_session_id);
        assert_eq!(preview.main_session_id, merged.main_session_id);
        let mut request: GenerateRequest =
            serde_json::from_value(data["guided_request"].clone()).unwrap();
        assert!(request.validate().is_ok());
        request.user_guidance = None;
        assert!(request.validate().is_err());
        request.user_guidance = Some(" ".into());
        assert!(request.validate().is_err());
        request.user_guidance = Some("a".repeat(8001));
        assert!(request.validate().is_err());
        assert!(serde_json::from_value::<PreviewRequest>(
            serde_json::json!({"session_id":"branch-1","summary":"injected"})
        )
        .is_err());
    }
}
