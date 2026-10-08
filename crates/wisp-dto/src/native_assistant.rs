//! Explicit native assistant and scheduler boundary. No WebView window rebinding.
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "wisp.native-assistant.v1";
pub const COMMANDS: &[&str] = &[
    "native_assistant_open",
    "native_assistant_workspace",
    "native_assistant_mutate",
];

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRequest {
    pub day: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct OpenResult {
    pub project_id: String,
    pub session_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Workspace {
    pub day: String,
    pub projects: Vec<Project>,
    pub plan: Vec<crate::ResearchAssistantPlanItem>,
    pub schedules: Vec<crate::ScheduleRecord>,
    pub runs: Vec<crate::ScheduleRunRecord>,
    pub daily_recap: crate::DailyRecapAutomation,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MutationRequest {
    pub operation: Operation,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TimerRequest {
    pub session_id: String,
    pub operation: TimerOperation,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum TimerOperation {
    Read,
    Set { expression: String },
    Cancel,
    SetEnabled { enabled: bool },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    CreateSchedule {
        project_id: String,
        name: String,
        prompt: String,
        interval_secs: i64,
        session_id: Option<String>,
        skill: Option<String>,
        start_at: Option<i64>,
    },
    SetTimer {
        project_id: String,
        session_id: String,
        expression: String,
    },
    SetEnabled {
        id: String,
        enabled: bool,
    },
    Delete {
        id: String,
    },
    RunNow {
        id: String,
    },
    SetDailyRecap {
        enabled: bool,
        time: String,
    },
    RunDailyRecap,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_assistant_fixture_roundtrips_the_shared_contract() {
        let value: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-assistant/v1/workspace.json"
        ))
        .unwrap();
        let _: OpenResult = serde_json::from_value(value["open"].clone()).unwrap();
        let workspace: Workspace = serde_json::from_value(value["workspace"].clone()).unwrap();
        assert_eq!(workspace.projects[0].id, "project-a");
        for operation in value["operations"].as_array().unwrap() {
            let typed: Operation = serde_json::from_value(operation.clone()).unwrap();
            assert_eq!(serde_json::to_value(typed).unwrap(), *operation);
        }
        assert!(serde_json::from_value::<Operation>(
            serde_json::json!({"action":"delete", "id":"x", "project_id":"hidden"})
        )
        .is_err());
    }
}
