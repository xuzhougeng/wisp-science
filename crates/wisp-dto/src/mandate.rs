//! Research mandates: a long-running responsibility an agent carries round by
//! round — a goal, how progress is measured, and what it may do on its own.
use serde::{Deserialize, Serialize};

/// `active` rounds fire when due; `paused` was stopped by the researcher;
/// `waiting` needs the researcher before the next round; `done` is closed.
pub const MANDATE_STATUSES: &[&str] = &["active", "paused", "waiting", "done"];

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MandateKpi {
    pub name: String,
    /// How the number is counted, so two readers get the same value.
    pub definition: String,
    pub target: String,
    /// The window the target applies to, e.g. "per week".
    pub period: String,
    /// The latest value a round reported.
    pub current: Option<String>,
}

impl MandateKpi {
    /// `target - current`, when both are plain numbers.
    pub fn gap(&self) -> Option<f64> {
        let number = |text: &str| text.trim().parse::<f64>().ok();
        Some(number(&self.target)? - number(self.current.as_deref()?)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MandateConstraints {
    /// What the agent may do without asking.
    pub autonomous: String,
    /// What the researcher reviews before it happens.
    pub review_first: String,
    /// When the agent should stop and ask for help.
    pub ask_for_help: String,
    /// Mutating tools ask for approval even where the project allows them.
    pub review_mutations: bool,
    /// Bounds on how soon and how late a round may schedule the next one.
    pub min_interval_secs: i64,
    pub max_interval_secs: i64,
    pub max_rounds_per_day: u32,
    /// Standing instructions the researcher confirmed from their feedback.
    pub notes: Vec<String>,
}

impl Default for MandateConstraints {
    fn default() -> Self {
        Self {
            autonomous: String::new(),
            review_first: String::new(),
            ask_for_help: String::new(),
            review_mutations: true,
            min_interval_secs: 15 * 60,
            max_interval_secs: 7 * 86_400,
            max_rounds_per_day: 6,
            notes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MandateRecord {
    pub id: String,
    pub project_id: String,
    /// The mandate's one conversation. `None` until its first round, or after
    /// that conversation was deleted.
    pub frame_id: Option<String>,
    pub name: String,
    pub goal: String,
    pub kpis: Vec<MandateKpi>,
    pub constraints: MandateConstraints,
    pub ends_at: Option<i64>,
    /// Cadence used when a round does not choose its own next time.
    pub interval_secs: i64,
    pub report_interval_secs: i64,
    pub next_report_at: Option<i64>,
    /// One of [`MANDATE_STATUSES`].
    pub status: String,
    pub next_run_at: i64,
    pub last_run_at: Option<i64>,
    /// A Run whose completion starts the next round early.
    pub wait_run_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// What the researcher fills in. The host owns ids, status and timing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MandateDraft {
    pub project_id: String,
    pub name: String,
    pub goal: String,
    pub kpis: Vec<MandateKpi>,
    pub constraints: MandateConstraints,
    pub ends_at: Option<i64>,
    pub interval_secs: i64,
    pub report_interval_secs: i64,
    /// First round. Absent means one interval from now.
    pub start_at: Option<i64>,
}

/// A mandate with what its card shows beside it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MandateOverview {
    pub mandate: MandateRecord,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kpi_gap_needs_two_numbers() {
        let mut kpi = MandateKpi {
            target: "10".into(),
            ..Default::default()
        };
        assert_eq!(kpi.gap(), None, "nothing reported yet");
        kpi.current = Some(" 7.5 ".into());
        assert_eq!(kpi.gap(), Some(2.5));
        kpi.target = "accepted".into();
        assert_eq!(kpi.gap(), None);
    }

    #[test]
    fn older_payloads_fill_in_constraint_defaults() {
        let constraints: MandateConstraints =
            serde_json::from_str(r#"{"autonomous":"read papers"}"#).unwrap();
        assert_eq!(constraints.autonomous, "read papers");
        assert!(constraints.review_mutations);
        assert_eq!(constraints.max_interval_secs, 7 * 86_400);
    }
}
