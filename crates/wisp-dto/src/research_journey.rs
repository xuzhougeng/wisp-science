//! Project research history. Unix timestamps are grouped into days by the UI's
//! local calendar; artifact references always identify immutable versions.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchJourney {
    pub entries: Vec<ResearchJourneyEntry>,
    pub truncated: bool,
    /// Mainline daily recaps in range; never part of `entries`.
    #[serde(default)]
    pub recaps: Vec<ResearchRecap>,
}

/// An AI-drafted summary of one local day. It stays a draft until the
/// researcher confirms it, and each item cites the recorded sources it
/// summarizes, so a recap never stands in for the records themselves.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchRecap {
    pub id: String,
    /// Local midnight of the summarized day.
    pub day_start: i64,
    /// `draft`, `confirmed` or `dismissed`.
    pub status: String,
    pub headline: String,
    pub done: Vec<ResearchRecapItem>,
    pub findings: Vec<ResearchRecapItem>,
    pub issues: Vec<ResearchRecapItem>,
    pub next: Vec<ResearchRecapItem>,
    pub sources: Vec<ResearchRecapSource>,
    pub model: String,
    pub generated_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchRecapItem {
    pub text: String,
    /// Indexes into `ResearchRecap::sources`.
    #[serde(default)]
    pub refs: Vec<usize>,
}

/// `kind` is `run`, `artifact` (an immutable version id), `session` or `record`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchRecapSource {
    pub kind: String,
    pub id: String,
    pub title: String,
}

/// The researcher's review of a recap draft; sources stay as generated.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchRecapEdit {
    pub id: String,
    pub status: String,
    pub headline: String,
    pub done: Vec<ResearchRecapItem>,
    pub findings: Vec<ResearchRecapItem>,
    pub issues: Vec<ResearchRecapItem>,
    pub next: Vec<ResearchRecapItem>,
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
