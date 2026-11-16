//! Automations: user-scheduled prompts and the built-in daily recap.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimerTurnRemoval {
    pub frame_id: String,
    pub first_user_index: usize,
    pub user_count: usize,
    pub base_epoch: i64,
}

/// Deliberately small grammar: a positive integer followed by m/h/d, then a prompt.
pub fn parse_timer_expression(expression: &str) -> Result<(i64, String), String> {
    let usage = "Usage: /timer 1h prompt (1m–365d; units: m, h, d).";
    let expression = expression.trim();
    let (duration, prompt) = expression.split_once(char::is_whitespace).ok_or(usage)?;
    let prompt = prompt.trim();
    let unit = duration.chars().last().ok_or(usage)?;
    let multiplier = match unit {
        'm' => 60,
        'h' => 3600,
        'd' => 86400,
        _ => return Err(usage.into()),
    };
    let digits = &duration[..duration.len() - 1];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || prompt.is_empty() {
        return Err(usage.into());
    }
    let interval = digits
        .parse::<i64>()
        .ok()
        .and_then(|n| n.checked_mul(multiplier))
        .filter(|n| (60..=365 * 86400).contains(n))
        .ok_or(usage)?;
    Ok((interval, prompt.into()))
}

#[cfg(test)]
mod timer_tests {
    use super::*;
    #[test]
    fn each_host_fires_its_own_rows_and_unmarked_replacing_timers_stay_the_webviews() {
        let row = |host: Option<&str>, replace: bool| ScheduleRecord {
            host: host.map(str::to_owned),
            replace_previous_turn: replace,
            ..Default::default()
        };
        let embedded = Some(EMBEDDED_SCHEDULE_HOST);
        // The embedded host's own rows: only it fires them.
        assert!(schedule_fired_by(&row(embedded, true), embedded));
        assert!(schedule_fired_by(&row(embedded, false), embedded));
        assert!(!schedule_fired_by(&row(embedded, true), None));
        assert!(!schedule_fired_by(&row(embedded, false), None));
        // An unmarked replacing timer is the WebView's alone.
        assert!(schedule_fired_by(&row(None, true), None));
        assert!(!schedule_fired_by(&row(None, true), embedded));
        // An unmarked task that adds a turn is fired by both, as before.
        assert!(schedule_fired_by(&row(None, false), None));
        assert!(schedule_fired_by(&row(None, false), embedded));
        // An unmarked row reads and writes as it always did.
        let old: ScheduleRecord = serde_json::from_value(serde_json::json!({
            "id": "s", "project_id": "p", "frame_id": null, "name": "n",
            "prompt": "p", "skill": null, "interval_secs": 60, "enabled": true,
            "next_run_at": 1, "last_run_at": null, "created_at": 1, "updated_at": 1
        }))
        .unwrap();
        assert_eq!(old.host, None);
        assert!(serde_json::to_value(&old).unwrap().get("host").is_none());
    }
    #[test]
    fn timer_grammar_preserves_prompt_and_rejects_invalid_intervals() {
        assert_eq!(
            parse_timer_expression(" 1h 当前进展如何\n检查日志 ").unwrap(),
            (3600, "当前进展如何\n检查日志".into())
        );
        assert_eq!(parse_timer_expression("30m status").unwrap().0, 1800);
        assert_eq!(parse_timer_expression("2d status").unwrap().0, 172800);
        for value in [
            "",
            "1h",
            "1h  ",
            "0h x",
            "-1h x",
            "+1h x",
            "1.5h x",
            "60s x",
            "366d x",
            "999999999999999999999h x",
            "小时 x",
        ] {
            assert!(parse_timer_expression(value).is_err(), "{value}");
        }
    }
}

/// The owner mark of a schedule the embedded host (the GPUI desktop's, or a
/// headless server) made and fires. A row without a mark is the WebView
/// desktop's, or was made before the hosts were told apart.
pub const EMBEDDED_SCHEDULE_HOST: &str = "embedded";

/// Whether the host named `host` (`None`: the WebView desktop; otherwise
/// its owner mark) fires `schedule`. Each host fires the rows it marked. A
/// row with no mark is the WebView's, except a conversation-bound or
/// unbound task that does not replace its previous turn: both hosts fired
/// those before rows were marked, and still do (claiming a slot is atomic,
/// so a time fires once).
pub fn schedule_fired_by(schedule: &ScheduleRecord, host: Option<&str>) -> bool {
    match (schedule.host.as_deref(), host) {
        (Some(owner), Some(host)) => owner == host,
        (Some(_), None) => false,
        (None, None) => true,
        (None, Some(_)) => !schedule.replace_previous_turn,
    }
}

/// An interval trigger that fires a prompt (with an optional skill) into a
/// chat session while the app is running.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleRecord {
    pub id: String,
    pub project_id: String,
    /// Target session. `None` creates a fresh session for every fire.
    pub frame_id: Option<String>,
    /// A conversation timer replaces its previous complete turn.
    #[serde(default)]
    pub replace_previous_turn: bool,
    pub name: String,
    pub prompt: String,
    pub skill: Option<String>,
    pub interval_secs: i64,
    pub enabled: bool,
    pub next_run_at: i64,
    pub last_run_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Which host fires it: `None` for the WebView desktop's rows (and rows
    /// made before hosts were told apart), [`EMBEDDED_SCHEDULE_HOST`] for
    /// the embedded host's. See [`schedule_fired_by`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
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

/// A memory as it read when a tidy proposal was made.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MemoryTidyEntry {
    pub id: String,
    pub content: String,
}

/// One change the memory tidy proposes. Nothing changes until it is applied.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MemoryTidyProposal {
    pub id: String,
    /// `merge`: replace `memories` with one memory reading `text`.
    /// `retire`: remove `memories`.
    pub action: String,
    pub memories: Vec<MemoryTidyEntry>,
    pub text: String,
    pub reason: String,
}

/// The built-in memory tidy: once a week it looks over the global habits and
/// proposes merging duplicates or retiring entries a newer one replaces.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MemoryTidyAutomation {
    pub enabled: bool,
    pub last_run_at: Option<i64>,
    pub error: Option<String>,
    pub running: bool,
    pub proposals: Vec<MemoryTidyProposal>,
}

impl Default for MemoryTidyAutomation {
    fn default() -> Self {
        Self {
            enabled: true,
            last_run_at: None,
            error: None,
            running: false,
            proposals: Vec::new(),
        }
    }
}
