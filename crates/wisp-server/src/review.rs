//! A review of a conversation, asked for by the user: the conversation's
//! model reads the latest of it as a reviewer and reports where the agent
//! claimed something its own tool output does not support.
//!
//! The rubric, the report's shape and the correction request are the
//! WebView desktop's (`src-tauri/src/review.rs`), which keeps its own copy
//! until that backend goes. This host has no reviewer of its own to choose,
//! runs no review by itself, and keeps no report: the caller gets it once.

use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use wisp_llm::{Message, Provider, Role};

/// What is shown of one tool's arguments or output, and of the whole.
const TOOL_LIMIT: usize = 4_000;
const TOTAL_LIMIT: usize = 80_000;
const FINDINGS: usize = 8;
/// A finding is a few sentences; a field longer than this was not one.
const FIELD_LIMIT: usize = 2_000;
const WAIT: std::time::Duration = std::time::Duration::from_secs(180);

const RUBRIC: &str = "\
You are a REVIEWER. You are given a transcript of another agent's working \
session — user turns, the agent's replies, and tool outputs (`[msg:N TOOL:name]`). \
Your job is to trace it and report where the agent fabricated a result, \
hallucinated a fact, or deviated from what it was asked to do.

Rules:
- Trace, don't recompute. If the agent claims a number or result, find the \
tool output that produced it and compare. A mismatch between a claim and the \
tool output it came from is a finding.
- Every finding must cite evidence from the transcript itself (quote the claim \
and the conflicting tool output). Never add facts from your own knowledge.
- A value you cannot trace inside this transcript is NOT a finding — it may \
come from earlier than the window you were given.
- A later explicit correction supersedes an earlier claim. Do not reflag a \
claim that the agent already corrected accurately.
- Do not restate what the agent did correctly. Only report problems.

Output one JSON object and nothing else:
{
  \"summary\": \"one sentence describing what was checked\",
  \"findings\": [
    {
      \"message_index\": 0,
      \"claim\": \"the exact problematic claim\",
      \"evidence\": \"the conflicting transcript evidence\",
      \"fix\": \"the smallest correction\",
      \"verdict\": \"warn or fail\",
      \"severity\": \"low, medium, or high\"
    }
  ]
}
Use the zero-based N from `[msg:N ...]` as message_index. If there are no \
problems, return an empty findings array. Order findings most severe first.";

/// The WebView desktop's keys: whether a conversation is reviewed after
/// each turn, and what a conversation that never chose starts with.
pub(crate) const DEFAULT_SETTING: &str = "auto_review_default_enabled";
pub(crate) fn setting(session: &str) -> String {
    format!("frame_auto_review:{session}")
}
/// The model profile reviews are made with; unset is the conversation's.
pub(crate) const MODEL_SETTING: &str = "desktop.review.model";

/// Whether a conversation is reviewed after each turn: its own choice, or
/// the default when it has made none.
pub(crate) async fn automatic(store: &wisp_store::Store, session: &str) -> bool {
    let saved = |key: String| async move { store.get_setting(&key).await.ok().flatten() };
    match saved(setting(session))
        .await
        .and_then(|value| value.parse::<bool>().ok())
    {
        Some(chosen) => chosen,
        None => automatic_by_default(store).await,
    }
}

pub(crate) async fn automatic_by_default(store: &wisp_store::Store) -> bool {
    store
        .get_setting(DEFAULT_SETTING)
        .await
        .ok()
        .flatten()
        .and_then(|value| value.parse::<bool>().ok())
        .unwrap_or(false)
}

/// Whether the latest turn is worth a review by itself: it used a tool, or
/// said a good deal. A greeting is not, and neither is a turn that stopped
/// to ask the user something, because that is not an answer yet.
pub(crate) fn worth_reviewing(messages: &[Message]) -> bool {
    let start = messages
        .iter()
        .rposition(|message| {
            message.role == Role::User
                && !wisp_store::is_compaction_checkpoint(&message.content.as_text())
        })
        .map_or(0, |at| at + 1);
    let turn = &messages[start..];
    let waits_for_user = turn
        .iter()
        .rev()
        .find(|message| message.role == Role::Assistant)
        .is_some_and(|message| {
            message.tool_calls.iter().any(|call| {
                matches!(
                    call.function.name.as_str(),
                    wisp_tools::ask_user::ASK_USER | wisp_tools::plan::PROPOSE_PLAN
                )
            })
        });
    if waits_for_user {
        return false;
    }
    let said = |message: &&Message| {
        message.role == Role::Assistant
            || (message.role == Role::Tool
                && message.tool_name.as_deref() == Some("attempt_completion"))
    };
    let used_a_tool = turn
        .iter()
        .any(|message| message.role == Role::Tool && !said(&message));
    let prose: usize = turn
        .iter()
        .filter(said)
        .map(|message| message.content.as_text().chars().count())
        .sum();
    used_a_tool || prose >= 600
}

/// Where the review that follows a turn stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The reviewer is reading.
    Reading,
    Done(Review),
    /// There is no report, and why.
    Failed(String),
}

/// What a reviewer found in a conversation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Review {
    /// One sentence on what was checked.
    #[serde(default)]
    pub summary: String,
    /// The problems, most severe first; none when nothing was found.
    #[serde(default)]
    pub findings: Vec<Finding>,
    /// The model that reviewed.
    #[serde(skip)]
    pub model: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Finding {
    #[serde(default)]
    pub claim: String,
    #[serde(default)]
    pub evidence: String,
    #[serde(default)]
    pub fix: String,
    /// `fail` or `warn`.
    #[serde(default)]
    pub verdict: String,
    /// `high`, `medium` or `low`.
    #[serde(default)]
    pub severity: String,
}

impl Review {
    /// The message that asks the agent to put right what was found. It is
    /// sent like any message, so the user can change it first.
    pub fn correction(&self) -> String {
        let findings = self
            .findings
            .iter()
            .enumerate()
            .map(|(index, finding)| {
                format!(
                    "{}. Claim: {}\nEvidence: {}\nRequired fix: {}",
                    index + 1,
                    finding.claim,
                    finding.evidence,
                    finding.fix
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        format!(
            "An independent reviewer found factual or traceability problems in your latest \
             answer. Correct the answer now. Use tools again if needed, preserve conclusions \
             that remain supported, and explicitly state each correction. Do not discuss the \
             review process itself.\n\n{findings}"
        )
    }
}

/// Has `model` review `messages`, a conversation's saved messages.
pub(crate) async fn ask(model: &dyn Provider, messages: &[Message]) -> Result<Review> {
    ensure!(
        messages
            .iter()
            .any(|message| message.role != Role::System && message.role != Role::User),
        "There is nothing to review yet"
    );
    let request = [Message::system(RUBRIC), Message::user(transcript(messages))];
    let reply = tokio::time::timeout(WAIT, model.complete(&request, &[]))
        .await
        .context("The reviewer did not answer within three minutes")??;
    let mut review = parse(&reply.content)?;
    review.model = model.model().to_owned();
    Ok(review)
}

/// The conversation as the reviewer reads it: numbered blocks for what the
/// user asked, what the agent said, and each tool's arguments and output,
/// with long tool text cut. When it is too long the oldest blocks go; the
/// numbers stay those of the whole conversation.
pub(crate) fn transcript(messages: &[Message]) -> String {
    let arguments: HashMap<&str, &str> = messages
        .iter()
        .flat_map(|message| &message.tool_calls)
        .map(|call| (call.id.as_str(), call.function.arguments.as_str()))
        .collect();
    let mut blocks = Vec::new();
    let mut add = |label: String, body: &str| {
        let body = body.trim();
        if !body.is_empty() {
            blocks.push(format!("[msg:{} {label}]\n{body}", blocks.len()));
        }
    };
    for message in messages {
        let text = message.content.as_text();
        match message.role {
            Role::System => {}
            Role::User => add("USER".into(), &text),
            Role::Assistant => add("ASSISTANT".into(), &text),
            Role::Tool => {
                let name = message.tool_name.as_deref().unwrap_or("tool");
                if name == "attempt_completion" {
                    add("ASSISTANT".into(), &text);
                    continue;
                }
                let given = message
                    .tool_call_id
                    .as_deref()
                    .and_then(|id| arguments.get(id))
                    .copied()
                    .unwrap_or_default();
                let output = clipped(&text, TOOL_LIMIT);
                let body = if given.trim().is_empty() {
                    format!("output:\n{output}")
                } else {
                    format!("args:\n{}\noutput:\n{output}", clipped(given, TOOL_LIMIT))
                };
                add(format!("TOOL:{name}"), &body);
            }
        }
    }
    // The latest blocks that fit; at least one, however long.
    let mut kept = Vec::new();
    let mut used = 0;
    for block in blocks.iter().rev() {
        let cost = block.chars().count() + 2;
        if !kept.is_empty() && used + cost > TOTAL_LIMIT {
            break;
        }
        used += cost;
        kept.push(block.as_str());
    }
    kept.reverse();
    kept.join("\n\n")
}

fn clipped(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let kept: String = chars.by_ref().take(limit).collect();
    match chars.count() {
        0 => kept,
        more => format!("{kept}\n[… {more} more characters]"),
    }
}

/// Reads the reviewer's reply: the JSON object in it, whatever is written
/// around it. Findings that do not say what, why and how to fix are left
/// out, because nothing could be done with them.
fn parse(raw: &str) -> Result<Review> {
    let Some(mut review) = objects(raw)
        .into_iter()
        .rev()
        .filter_map(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .find(|value| value.get("findings").is_some() || value.get("summary").is_some())
        .and_then(|value| serde_json::from_value::<Review>(value).ok())
    else {
        bail!("The reviewer did not return a report");
    };
    review.findings.retain(|finding| {
        !finding.claim.trim().is_empty()
            && !finding.evidence.trim().is_empty()
            && !finding.fix.trim().is_empty()
            && !finding.verdict.eq_ignore_ascii_case("pass")
    });
    review.findings.truncate(FINDINGS);
    for finding in &mut review.findings {
        for field in [&mut finding.claim, &mut finding.evidence, &mut finding.fix] {
            *field = clipped(field.trim(), FIELD_LIMIT);
        }
        finding.verdict = match finding.verdict.to_ascii_lowercase().as_str() {
            "fail" => "fail",
            _ => "warn",
        }
        .into();
        finding.severity = match finding.severity.to_ascii_lowercase().as_str() {
            "high" => "high",
            "medium" | "med" => "medium",
            _ => "low",
        }
        .into();
    }
    review.summary = clipped(review.summary.trim(), FIELD_LIMIT);
    if review.summary.is_empty() {
        review.summary = match review.findings.len() {
            0 => "No issues found.".into(),
            count => format!("{count} finding(s) require correction."),
        };
    }
    Ok(review)
}

/// Every top-level `{…}` in `raw`, in order: a model may write prose with
/// braces in it before or after the report.
fn objects(raw: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let (mut depth, mut start, mut quoted, mut escaped) = (0usize, 0, false, false);
    for (at, ch) in raw.char_indices() {
        if quoted {
            match ch {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => quoted = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' if depth > 0 => quoted = true,
            '{' => {
                if depth == 0 {
                    start = at;
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    found.push(&raw[start..=at]);
                }
            }
            _ => {}
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_llm::{FunctionCall, ToolCall};

    fn calling(id: &str, name: &str, arguments: &str) -> Message {
        let mut message = Message::assistant("Running it.");
        message.tool_calls = vec![ToolCall {
            id: id.into(),
            kind: "function".into(),
            function: FunctionCall {
                name: name.into(),
                arguments: arguments.into(),
            },
        }];
        message
    }

    #[test]
    fn the_reviewer_reads_numbered_blocks_with_each_tools_arguments_and_output() {
        let messages = [
            Message::system("You are Wisp."),
            Message::user("count the reads"),
            calling("c1", "shell", r#"{"cmd":"wc -l reads.fq"}"#),
            Message::tool("c1", "shell", "400 reads.fq"),
            Message::tool("c2", "attempt_completion", "There are 100 reads."),
        ];
        assert_eq!(
            transcript(&messages),
            "[msg:0 USER]\ncount the reads\n\n\
             [msg:1 ASSISTANT]\nRunning it.\n\n\
             [msg:2 TOOL:shell]\nargs:\n{\"cmd\":\"wc -l reads.fq\"}\noutput:\n400 reads.fq\n\n\
             [msg:3 ASSISTANT]\nThere are 100 reads."
        );
        // A long tool output is cut, and says by how much.
        let long = [
            Message::user("go"),
            Message::tool("lost", "read", "y".repeat(TOOL_LIMIT + 9)),
        ];
        assert!(transcript(&long).ends_with(&format!(
            "output:\n{}\n[… 9 more characters]",
            "y".repeat(TOOL_LIMIT)
        )));
        // Too long in all: the oldest blocks go, the numbers stay.
        let many: Vec<_> = (0..40)
            .map(|index| Message::user(format!("{index} {}", "z".repeat(3_000))))
            .collect();
        let text = transcript(&many);
        assert!(text.chars().count() <= TOTAL_LIMIT);
        assert!(!text.contains("[msg:0 USER]"));
        assert!(text.contains("[msg:39 USER]\n39 z"));
    }

    #[test]
    fn a_report_is_read_out_of_whatever_surrounds_it() {
        let raw = r#"Here {is} my report:
```json
{"summary": " Checked the read count. ", "findings": [
  {"message_index": 3, "claim": "There are 100 reads", "evidence": "wc printed \"400\" {lines}",
   "fix": "Say 400 lines, 100 reads", "verdict": "FAIL", "severity": "High"},
  {"claim": "fine", "evidence": "", "fix": "none", "verdict": "warn"},
  {"claim": "ok", "evidence": "x", "fix": "y", "verdict": "pass"},
  {"claim": "Vague", "evidence": "x", "fix": "y", "verdict": "maybe", "severity": "med"},
  {"claim": "Minor", "evidence": "x", "fix": "y"}
]}
```
Hope {that} helps."#;
        let review = parse(raw).unwrap();
        assert_eq!(review.summary, "Checked the read count.");
        let read: Vec<_> = review
            .findings
            .iter()
            .map(|f| (f.claim.as_str(), f.verdict.as_str(), f.severity.as_str()))
            .collect();
        assert_eq!(
            read,
            [
                ("There are 100 reads", "fail", "high"),
                ("Vague", "warn", "medium"),
                ("Minor", "warn", "low"),
            ]
        );
        assert_eq!(review.findings[0].evidence, "wc printed \"400\" {lines}");

        // Nothing found, with or without a summary.
        let clean = parse(r#"{"findings": []}"#).unwrap();
        assert_eq!(clean.summary, "No issues found.");
        assert!(clean.findings.is_empty());
        // No more findings than can be read.
        let finding = r#"{"claim":"c","evidence":"e","fix":"f"}"#;
        let many = format!(r#"{{"findings":[{}]}}"#, vec![finding; 12].join(","));
        let review = parse(&many).unwrap();
        assert_eq!(review.findings.len(), FINDINGS);
        assert_eq!(review.summary, "8 finding(s) require correction.");
        // Not a report at all.
        for raw in ["All good!", "{not json}", r#"{"verdict":"pass"}"#, "{"] {
            assert!(parse(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn only_a_turn_with_something_to_check_is_reviewed_by_itself() {
        let asked = Message::user("count the reads");
        // A tool ran.
        assert!(worth_reviewing(&[
            asked.clone(),
            calling("c1", "shell", "{}"),
            Message::tool("c1", "shell", "400"),
            Message::assistant("There are 100."),
        ]));
        // A long answer without tools.
        assert!(worth_reviewing(&[
            asked.clone(),
            Message::assistant("x".repeat(600)),
        ]));
        // A short one, a completion that only says so, or nothing at all.
        assert!(!worth_reviewing(&[
            asked.clone(),
            Message::assistant("Hello!")
        ]));
        assert!(!worth_reviewing(&[
            asked.clone(),
            Message::tool("c9", "attempt_completion", "Done."),
        ]));
        assert!(!worth_reviewing(&[]));
        // Stopped to ask: not an answer yet, whatever ran before.
        assert!(!worth_reviewing(&[
            asked.clone(),
            calling("c1", "shell", "{}"),
            Message::tool("c1", "shell", "400"),
            calling("c2", "ask_user", "{}"),
            Message::tool("c2", "ask_user", "waiting"),
        ]));
        // Only the latest turn counts: an earlier one's tools do not.
        assert!(!worth_reviewing(&[
            asked.clone(),
            calling("c1", "shell", "{}"),
            Message::tool("c1", "shell", "400"),
            Message::assistant("There are 100."),
            Message::user("thanks"),
            Message::assistant("Any time."),
        ]));
    }

    #[test]
    fn the_correction_lists_what_to_put_right() {
        let review = Review {
            summary: "Checked.".into(),
            findings: vec![
                Finding {
                    claim: "100 reads".into(),
                    evidence: "wc printed 400".into(),
                    fix: "Say 400 lines".into(),
                    verdict: "fail".into(),
                    severity: "high".into(),
                },
                Finding {
                    claim: "No adapters".into(),
                    evidence: "fastqc flagged adapters".into(),
                    fix: "Report them".into(),
                    verdict: "warn".into(),
                    severity: "low".into(),
                },
            ],
            model: "m".into(),
        };
        let text = review.correction();
        assert!(text.starts_with("An independent reviewer found"));
        assert!(text.ends_with(
            "1. Claim: 100 reads\nEvidence: wc printed 400\nRequired fix: Say 400 lines\n\n\
             2. Claim: No adapters\nEvidence: fastqc flagged adapters\nRequired fix: Report them"
        ));
    }
}
