//! Wording and small decisions for the conversation's controls: reasoning
//! effort, token usage, approvals and the model's questions. No graphics.
use wisp_dto::{PendingToolApproval, native_conversations::Item};
use wisp_server::desktop::{TokenUsage, tool_question};

/// Reasoning efforts in the order they are offered. The ids are the host's.
pub const EFFORTS: [(&str, &str); 8] = [
    ("", "Default"),
    ("none", "None"),
    ("minimal", "Minimal"),
    ("low", "Low"),
    ("medium", "Medium"),
    ("high", "High"),
    ("xhigh", "Extra high"),
    ("max", "Max"),
];

pub fn effort_label(id: &str) -> &'static str {
    EFFORTS
        .iter()
        .find(|(effort, _)| *effort == id)
        .map_or("Default", |(_, label)| label)
}

/// A token count short enough for a status line: 950, 19.8k, 1.2M.
pub fn compact(count: u64) -> String {
    match count {
        0..=999 => count.to_string(),
        1_000..=999_949 => format!("{:.1}k", count as f64 / 1_000.0),
        _ => format!("{:.1}M", count as f64 / 1_000_000.0),
    }
}

/// What the latest turn cost, leaving out parts that were zero.
pub fn usage_line(usage: &TokenUsage) -> String {
    let mut parts = vec![
        format!("Input {}", compact(usage.input)),
        format!("output {}", compact(usage.output)),
    ];
    if usage.reasoning > 0 {
        parts.push(format!("thinking {}", compact(usage.reasoning)));
    }
    if usage.cached > 0 {
        parts.push(format!("cached {}", compact(usage.cached)));
    }
    format!("{} tokens", parts.join(" · "))
}

/// How full the model's context is, as a whole percentage. A context that
/// holds anything at all is never shown as 0%.
pub fn context_percent(usage: &TokenUsage) -> Option<u8> {
    if usage.max_context == 0 {
        return None;
    }
    let percent = usage.context_tokens as f64 * 100.0 / usage.max_context as f64;
    Some(if usage.context_tokens > 0 {
        percent.round().clamp(1.0, 100.0) as u8
    } else {
        0
    })
}

/// Only the registry's standard question can be answered for the whole
/// conversation; the host refuses anything else.
pub fn can_allow_for_conversation(approval: &PendingToolApproval) -> bool {
    approval.message == tool_question(&approval.tool)
}

/// A question the model asked through `ask_user`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub question: String,
    /// Label and optional one-line description.
    pub options: Vec<(String, String)>,
    pub allow_freeform: bool,
}

/// Reads the question out of an `ask_user` row. A call that failed or whose
/// result is not the card body is left to show as an ordinary tool row.
pub fn question(item: &Item) -> Option<Question> {
    if item.role != "tool"
        || item.tool_name.as_deref() != Some("ask_user")
        || item.ok == Some(false)
    {
        return None;
    }
    let body: serde_json::Value = serde_json::from_str(&item.text).ok()?;
    let question = body.get("question")?.as_str()?.trim();
    if question.is_empty() {
        return None;
    }
    let options = body
        .get("options")
        .and_then(|options| options.as_array())
        .map(|options| {
            options
                .iter()
                .filter_map(|option| {
                    let label = option.get("label")?.as_str()?.trim();
                    let description = option
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or_default()
                        .trim();
                    (!label.is_empty()).then(|| (label.to_owned(), description.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Question {
        question: question.to_owned(),
        options,
        allow_freeform: body
            .get("allow_freeform")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
    })
}

/// Whether the question in row `index` is still waiting: it is the last thing
/// said, and the turn that asked it has ended.
pub fn awaiting_answer(rows: &[Item], index: usize, running: bool) -> bool {
    !running
        && rows.get(index).and_then(question).is_some()
        && rows[index + 1..]
            .iter()
            .all(|row| row.role == "assistant" && row.text.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(role: &str, tool: Option<&str>, text: &str, ok: Option<bool>) -> Item {
        serde_json::from_value(json!({
            "role": role, "text": text, "tool_name": tool, "input": null,
            "ok": ok, "status": null
        }))
        .unwrap()
    }

    fn ask(body: serde_json::Value) -> Item {
        item("tool", Some("ask_user"), &body.to_string(), Some(true))
    }

    #[test]
    fn efforts_are_exactly_the_ones_the_host_accepts() {
        let ids: Vec<_> = EFFORTS.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, wisp_server::desktop::EFFORTS);
        assert_eq!(effort_label(""), "Default");
        assert_eq!(effort_label("xhigh"), "Extra high");
        assert_eq!(effort_label("unknown"), "Default");
    }

    #[test]
    fn usage_reads_at_a_glance() {
        assert_eq!(compact(0), "0");
        assert_eq!(compact(950), "950");
        assert_eq!(compact(19_840), "19.8k");
        assert_eq!(compact(1_250_000), "1.2M");
        let mut usage = TokenUsage {
            input: 19_840,
            output: 35,
            context_tokens: 2_560,
            max_context: 64_000,
            ..Default::default()
        };
        assert_eq!(usage_line(&usage), "Input 19.8k · output 35 tokens");
        assert_eq!(context_percent(&usage), Some(4));
        usage.reasoning = 45;
        usage.cached = 17_000;
        assert_eq!(
            usage_line(&usage),
            "Input 19.8k · output 35 · thinking 45 · cached 17.0k tokens"
        );
        // Nearly empty is still not empty; over-full is capped.
        usage.context_tokens = 10;
        assert_eq!(context_percent(&usage), Some(1));
        usage.context_tokens = 90_000;
        assert_eq!(context_percent(&usage), Some(100));
        usage.context_tokens = 0;
        assert_eq!(context_percent(&usage), Some(0));
        usage.max_context = 0;
        assert_eq!(context_percent(&usage), None);
    }

    #[test]
    fn only_the_standard_question_can_be_allowed_for_the_conversation() {
        let approval = |message: &str| PendingToolApproval {
            approval_id: "a".into(),
            frame_id: "s".into(),
            message: message.into(),
            tool: "write".into(),
            preview: String::new(),
        };
        assert!(can_allow_for_conversation(&approval("Run tool 'write'?")));
        assert!(!can_allow_for_conversation(&approval("Run tool 'shell'?")));
        assert!(!can_allow_for_conversation(&approval(
            "This command deletes files. Run it?"
        )));
    }

    #[test]
    fn a_question_card_is_read_from_the_tool_result() {
        let asked = ask(json!({
            "v": 1, "question": " Which reference genome? ",
            "options": [
                {"label": "GRCh38", "description": "Current"},
                {"label": " T2T "},
                {"label": ""}
            ],
            "allow_freeform": false, "note": "…"
        }));
        assert_eq!(
            question(&asked),
            Some(Question {
                question: "Which reference genome?".into(),
                options: vec![
                    ("GRCh38".into(), "Current".into()),
                    ("T2T".into(), String::new())
                ],
                allow_freeform: false,
            })
        );
        // Freeform is the default, and options are optional.
        let open = question(&ask(json!({"question": "Why?"}))).unwrap();
        assert!(open.allow_freeform && open.options.is_empty());
        // Not a card: another tool, a failed call, text that is not the body.
        assert!(
            question(&item(
                "tool",
                Some("shell"),
                "{\"question\":\"x\"}",
                Some(true)
            ))
            .is_none()
        );
        assert!(
            question(&item(
                "tool",
                Some("ask_user"),
                "{\"question\":\"x\"}",
                Some(false)
            ))
            .is_none()
        );
        assert!(
            question(&item(
                "tool",
                Some("ask_user"),
                "ask_user error: …",
                Some(true)
            ))
            .is_none()
        );
        assert!(question(&ask(json!({"question": "  "}))).is_none());
    }

    #[test]
    fn a_question_waits_until_something_is_said_after_it() {
        let asked = ask(json!({"question": "Which genome?"}));
        let rows = [item("user", None, "Align these", None), asked.clone()];
        assert!(awaiting_answer(&rows, 1, false));
        // Still being asked while the turn runs.
        assert!(!awaiting_answer(&rows, 1, true));
        assert!(!awaiting_answer(&rows, 0, false));
        assert!(!awaiting_answer(&rows, 5, false));
        let answered = [asked.clone(), item("user", None, "GRCh38", None)];
        assert!(!awaiting_answer(&answered, 0, false));
        // An empty trailing assistant row is not an answer.
        let trailing = [asked, item("assistant", None, " ", None)];
        assert!(awaiting_answer(&trailing, 0, false));
    }
}
