//! Versioned project-browser query and explicit command protocol for native clients.

use serde::{Deserialize, Serialize};

use crate::{ProjectSummary, RecentSession};

pub const SCHEMA: &str = "wisp.project-browser.v1";

#[derive(Serialize, Deserialize)]
pub struct Request {
    pub schema: String,
    pub id: String,
    #[serde(flatten)]
    pub command: Command,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    ListProjects,
    SetProjectStarred {
        project_id: String,
        starred: bool,
    },
    ListSessions {
        project_id: Option<String>,
    },
    GetTranscript {
        project_id: String,
        session_id: String,
        before_seq: Option<i64>,
    },
    Capabilities,
}

#[derive(Serialize, Deserialize)]
pub struct Response {
    pub schema: String,
    pub id: Option<String>,
    #[serde(flatten)]
    pub reply: Reply,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Projects {
        projects: Vec<ProjectSummary>,
        activity_source: ActivitySource,
    },
    Transcript {
        messages: Vec<BrowserMessage>,
        next_before_seq: Option<i64>,
    },
    Sessions {
        sessions: Vec<RecentSession>,
        activity_source: ActivitySource,
    },
    Capabilities {
        commands: Vec<String>,
        read_only: bool,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivitySource {
    /// No live runtime snapshot: counts reflect saved replies only.
    PersistedOnly,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    UnsupportedSchema,
    QueryFailed,
    WriteDisabled,
    CommandFailed,
}

/// Saved transcript text for native navigation; tool execution remains in the host.
#[derive(Serialize, Deserialize)]
pub struct BrowserMessage {
    pub seq: i64,
    pub role: String,
    pub text: String,
    pub tool_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relationship_fixture_preserves_links_and_older_sessions_stay_compatible() {
        let response: Response = serde_json::from_str(include_str!(
            "../../../contracts/project-browser/v1/relationships.json"
        ))
        .unwrap();
        let Reply::Sessions { sessions, .. } = response.reply else {
            panic!("Expected sessions")
        };
        assert_eq!(sessions[1].branched_from.as_deref(), Some("main"));
        assert_eq!(sessions[2].branch_state.as_deref(), Some("merged"));
        assert_eq!(sessions[3].branch_state.as_deref(), Some("orphaned"));
        assert!(sessions[3].branched_from.is_none());
        assert_eq!(sessions[4].dispatched_from.as_deref(), Some("main"));
        let main = serde_json::to_value(&sessions[0]).unwrap();
        assert!(main.get("branched_from").is_none());
        let old: Response = serde_json::from_str(include_str!(
            "../../../contracts/project-browser/v1/sessions.json"
        ))
        .unwrap();
        let Reply::Sessions { sessions, .. } = old.reply else {
            panic!("Expected sessions")
        };
        assert!(sessions[0].branched_from.is_none());
        assert!(sessions[0].branch_state.is_none());
    }
}
