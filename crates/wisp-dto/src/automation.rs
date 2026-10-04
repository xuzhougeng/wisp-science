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
