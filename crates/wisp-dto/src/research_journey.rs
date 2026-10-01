//! Project research history. Unix timestamps are grouped into days by the UI's
//! local calendar; artifact references always identify immutable versions.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchJourney {
    pub entries: Vec<ResearchJourneyEntry>,
    pub truncated: bool,
}

/// One project's mainline history in the home calendar. A failed project stays
/// visible as an error instead of making an incomplete calendar look empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchCalendarProject {
    pub project_id: String,
    pub history: ResearchJourney,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchJourneyEntry {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub occurred_at: i64,
    pub recorded_at: i64,
    pub source_id: String,
    pub frame_id: Option<String>,
    pub status: String,
    pub content_type: String,
    pub version_number: Option<i64>,
    pub source_discarded: bool,
    pub manual: bool,
    /// The run itself for run rows, the producing run for outputs.
    #[serde(default)]
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchJournalInput {
    pub title: String,
    pub body: String,
    pub category: String,
    pub occurred_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchJourneySource {
    pub run_id: Option<String>,
    pub run_title: String,
    pub run_status: String,
    pub context_id: String,
    pub generated_at: Option<i64>,
    pub inputs: Vec<ResearchJourneyInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchJourneyInput {
    pub title: String,
    pub role: String,
    pub version_id: Option<String>,
    pub confidence: String,
}
