//! The helpers a conversation sent off: each `explore` call the model
//! made, with what it was asked and what it found. Like the notebook it is
//! a reading of the transcript, not something saved on its own.
use wisp_dto::native_conversations::Item;

use crate::present::{self, Tone};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Helper {
    /// From 1, in the order they were sent.
    pub number: usize,
    /// What it was asked to find out.
    pub question: String,
    /// What the caller cared about most, if it said.
    pub focus: String,
    /// What it reported back; empty while it is still out.
    pub findings: String,
    /// How it went, as the transcript's tool row says.
    pub status: Option<(String, Tone)>,
    /// The transcript row it comes from.
    pub row: usize,
}

/// The helpers of a transcript, in order.
pub fn helpers(rows: &[Item], turn_running: bool) -> Vec<Helper> {
    let mut found = Vec::new();
    for (row, item) in rows.iter().enumerate() {
        if item.role != "tool" || item.tool_name.as_deref() != Some("explore") {
            continue;
        }
        let arguments: serde_json::Value = item
            .input
            .as_deref()
            .and_then(|input| serde_json::from_str(input).ok())
            .unwrap_or_default();
        let text = |key: &str| {
            arguments
                .get(key)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .unwrap_or_default()
                .to_owned()
        };
        found.push(Helper {
            number: found.len() + 1,
            question: text("question"),
            focus: text("focus"),
            findings: item.text.trim().to_owned(),
            status: present::tool_status(item, turn_running),
            row,
        });
    }
    found
}

/// The line under the tab's title.
pub fn summary(helpers: &[Helper], has_conversation: bool) -> String {
    use crate::i18n::{Locale, current, tr};
    match (has_conversation, helpers.len()) {
        (false, _) => tr("Open a conversation to see the helpers it sent off.").to_owned(),
        (true, 0) => tr(
            "No helper has been sent off in this conversation. The model sends one when a \
             question needs a search of its own.",
        )
        .to_owned(),
        (true, count) => match current() {
            Locale::Zh => format!("本会话派出了 {count} 个帮手"),
            Locale::En if count == 1 => "1 helper was sent off in this conversation".to_owned(),
            Locale::En => format!("{count} helpers were sent off in this conversation"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(value: serde_json::Value) -> Item {
        let mut whole = json!({"role": "tool", "text": ""});
        for (key, changed) in value.as_object().unwrap() {
            whole[key] = changed.clone();
        }
        serde_json::from_value(whole).unwrap()
    }

    #[test]
    fn each_explore_call_is_a_helper_with_its_question_and_findings() {
        let rows = [
            row(json!({"role": "user", "text": "where is the aligner configured?"})),
            row(json!({"tool_name": "shell", "input": "{\"cmd\":\"ls\"}", "text": "a"})),
            row(json!({
                "tool_name": "explore",
                "input": "{\"question\":\" Where is the aligner set up? \",\"focus\":\"config files\"}",
                "text": " In config/align.yml. ",
                "ok": true,
                "duration_ms": 4200
            })),
            row(json!({
                "tool_name": "explore",
                "input": "{\"question\":\"Which samples failed QC?\"}",
                "status": "running"
            })),
            // Arguments that cannot be read still make a helper.
            row(json!({"tool_name": "explore", "input": "not json", "ok": false})),
        ];
        let found = helpers(&rows, true);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].number, 1);
        assert_eq!(found[0].question, "Where is the aligner set up?");
        assert_eq!(found[0].focus, "config files");
        assert_eq!(found[0].findings, "In config/align.yml.");
        assert_eq!(found[0].row, 2);
        assert_eq!(
            found[0].status,
            Some(("Done · 4.2 s".into(), Tone::Success))
        );
        // Still out: nothing found yet.
        assert_eq!(found[1].findings, "");
        assert_eq!(found[1].status, Some(("Running".into(), Tone::Active)));
        assert_eq!(found[2].question, "");
        assert_eq!(found[2].status, Some(("Failed".into(), Tone::Danger)));
        // The turn over, one that never came back was interrupted.
        assert_eq!(
            helpers(&rows, false)[1].status,
            Some(("Interrupted".into(), Tone::Neutral))
        );
    }

    #[test]
    fn the_summary_counts_the_helpers_or_says_why_there_are_none() {
        let one = helpers(
            &[row(
                json!({"tool_name": "explore", "input": "{\"question\":\"q\"}"}),
            )],
            false,
        );
        assert_eq!(
            summary(&one, true),
            "1 helper was sent off in this conversation"
        );
        assert!(summary(&[], true).starts_with("No helper has been sent off"));
        assert!(summary(&[], false).starts_with("Open a conversation"));
    }
}
