//! Native scratch chat. Opening does not take a project id. Closing names the
//! scratch project and does not restore another project.
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "wisp.native-scratch.v1";

pub const COMMANDS: &[&str] = &["native_scratch_open", "native_scratch_close"];

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenRequest {}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ScratchSession {
    pub project_id: String,
    pub session_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratch_fixtures_do_not_restore_a_project() {
        let open: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-scratch/v1/open.json"
        ))
        .unwrap();
        assert_eq!(open["schema"], SCHEMA);
        assert!(open["project_id"].is_null());
        assert_eq!(open["command"], "native_scratch_open");
        let _: OpenRequest = serde_json::from_value(open["args"].clone()).unwrap();
        let session: ScratchSession = serde_json::from_value(open["result"].clone()).unwrap();
        assert!(session.project_id.starts_with("scratch:"));
        assert!(!session.session_id.is_empty());
        let mut extra = open["args"].clone();
        extra["active_window"] = serde_json::json!("main");
        assert!(serde_json::from_value::<OpenRequest>(extra).is_err());

        let close: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-scratch/v1/close.json"
        ))
        .unwrap();
        assert_eq!(close["command"], "native_scratch_close");
        assert_eq!(close["project_id"], session.project_id);
        assert_eq!(close["result"], true);
    }
}
