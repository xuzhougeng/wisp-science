//! Automations: user-scheduled prompts and the built-in daily recap.
use serde::{Deserialize, Serialize};

/// An interval trigger that fires a prompt (with an optional skill) into a
/// chat session while the app is running.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleRecord {
    pub id: String,
    pub project_id: String,
    /// Target session. `None` creates a fresh session for every fire.
    pub frame_id: Option<String>,
    pub name: String,
    pub prompt: String,
    pub skill: Option<String>,
    pub interval_secs: i64,
    pub enabled: bool,
    pub next_run_at: i64,
    pub last_run_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleRunRecord {
    pub id: String,
    pub schedule_id: String,
    pub frame_id: Option<String>,
    /// `fired` or `failed`.
    pub status: String,
    pub error: Option<String>,
    pub fired_at: i64,
}

/// The built-in daily recap: after `time` (local `HH:MM`) each day, draft a
/// recap for every project day with activity that has none yet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DailyRecapAutomation {
    pub enabled: bool,
    pub time: String,
    #[serde(default)]
    pub last_run_at: Option<i64>,
    #[serde(default)]
    pub drafted: usize,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub running: bool,
}

impl Default for DailyRecapAutomation {
    fn default() -> Self {
        Self {
            enabled: true,
            time: "09:00".into(),
            last_run_at: None,
            drafted: 0,
            error: None,
            running: false,
        }
    }
}
