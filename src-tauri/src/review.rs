//! Independent session review: serialize a frame into traceable blocks, parse
//! a structured reviewer report, and turn findings into one correction prompt.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;
use wisp_llm::{Message, Role};

/// Reviewer system prompt. Self-authored (an Apache-2.0 repo must not bundle
/// the upstream proprietary REVIEWER prompt) — captures the same job: trace the
/// transcript, don't recompute; a finding needs transcript evidence.
pub const REVIEWER_RUBRIC: &str = "\
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewFinding {
    #[serde(default)]
    pub message_index: usize,
    #[serde(default)]
    pub claim: String,
    #[serde(default)]
    pub evidence: String,
    #[serde(default)]
    pub fix: String,
    #[serde(default)]
    pub verdict: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default = "open_status")]
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewReport {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<ReviewFinding>,
    #[serde(default)]
    pub reviewer_model: String,
    #[serde(default)]
    pub reviewer_effort: String,
}

fn open_status() -> String {
    "open".into()
}

impl ReviewReport {
    pub fn has_findings(&self) -> bool {
        !self.findings.is_empty()
    }

    pub fn set_status(&mut self, status: &str) {
        for finding in &mut self.findings {
            finding.status = status.to_string();
        }
    }
}

/// Per-tool-result char cap: tool dumps (CSVs, stack traces) are the p90 size
/// driver; the reviewer traces claims, it doesn't need the full dump.
const PER_TOOL_CAP: usize = 4_000;
/// Whole-transcript char cap, kept from the tail (most recent turns).
// ponytail: char-based tail window; a huge session is reviewed from its recent
// end only. Upgrade path: per-checkpoint windowing if long-session recall matters.
const TOTAL_CAP: usize = 80_000;

/// Render persisted messages as blocks whose indices match the reloaded UI
/// transcript: reasoning and assistant prose are separate; tool arguments are
/// paired with their result instead of becoming an invisible assistant row.
pub fn serialize_transcript(msgs: &[Message]) -> String {
    let calls: HashMap<&str, (&str, &str)> = msgs
        .iter()
        .flat_map(|message| message.tool_calls.iter())
        .map(|call| {
            (
                call.id.as_str(),
                (
                    call.function.name.as_str(),
                    call.function.arguments.as_str(),
                ),
            )
        })
        .collect();
    let mut blocks: Vec<String> = Vec::new();
    let mut index = 0usize;
    for m in msgs {
        match m.role {
            Role::System => {}
            Role::User => push_block(&mut blocks, &mut index, "USER", &m.content.as_text()),
            Role::Assistant => {
                if let Some(reasoning) = m.reasoning.as_deref() {
                    push_block(&mut blocks, &mut index, "THINKING", reasoning);
                }
                push_block(&mut blocks, &mut index, "ASSISTANT", &m.content.as_text());
            }
            Role::Tool => {
                let name = m.tool_name.as_deref().unwrap_or("tool");
                if name == "attempt_completion" {
                    push_block(&mut blocks, &mut index, "ASSISTANT", &m.content.as_text());
                    continue;
                }
                let arguments = m
                    .tool_call_id
                    .as_deref()
                    .and_then(|id| calls.get(id))
                    .map(|(_, arguments)| *arguments)
                    .unwrap_or("");
                let body = if arguments.is_empty() {
                    format!("output:\n{}", truncate(&m.content.as_text(), PER_TOOL_CAP))
                } else {
                    format!(
                        "input:\n{}\noutput:\n{}",
                        truncate(arguments, PER_TOOL_CAP),
                        truncate(&m.content.as_text(), PER_TOOL_CAP)
                    )
                };
                push_block(&mut blocks, &mut index, &format!("TOOL:{name}"), &body);
            }
        }
    }

    // Keep the most recent blocks that fit under TOTAL_CAP.
    let mut kept_rev: Vec<&str> = Vec::new();
    let mut used = 0usize;
    for b in blocks.iter().rev() {
        let cost = b.len() + 2;
        if !kept_rev.is_empty() && used + cost > TOTAL_CAP {
            break;
        }
        used += cost;
        kept_rev.push(b);
    }
    let truncated = kept_rev.len() < blocks.len();
    kept_rev.reverse();

    let mut out = String::new();
    if truncated {
        out.push_str("[…earlier transcript truncated…]\n\n");
    }
    out.push_str(&kept_rev.join("\n\n"));
    out
}

fn push_block(blocks: &mut Vec<String>, index: &mut usize, label: &str, body: &str) {
    if body.trim().is_empty() {
        return;
    }
    blocks.push(format!("[msg:{} {}]\n{}", *index, label, body));
    *index += 1;
}

/// Parse the reviewer's JSON, tolerating a single Markdown fence while keeping
/// the accepted finding vocabulary small and predictable for the UI.
pub fn parse_report(raw: &str, reviewer_model: &str) -> Result<ReviewReport, String> {
    let start = raw
        .find('{')
        .ok_or_else(|| "Reviewer returned no JSON object.".to_string())?;
    let end = raw
        .rfind('}')
        .filter(|end| *end >= start)
        .ok_or_else(|| "Reviewer returned incomplete JSON.".to_string())?;
    let mut report: ReviewReport = serde_json::from_str(&raw[start..=end])
        .map_err(|e| format!("Invalid reviewer JSON: {e}"))?;
    report.id = Uuid::new_v4().to_string();
    report.reviewer_model = reviewer_model.to_string();
    report.findings.truncate(8);
    report.findings.retain(|finding| {
        !finding.claim.trim().is_empty()
            && !finding.evidence.trim().is_empty()
            && !finding.fix.trim().is_empty()
            && !finding.verdict.eq_ignore_ascii_case("pass")
    });
    for finding in &mut report.findings {
        finding.verdict = match finding.verdict.to_ascii_lowercase().as_str() {
            "fail" => "fail",
            "inconclusive" => "inconclusive",
            _ => "warn",
        }
        .into();
        finding.severity = match finding.severity.to_ascii_lowercase().as_str() {
            "high" => "high",
            "medium" | "med" => "medium",
            _ => "low",
        }
        .into();
        finding.status = "open".into();
    }
    if report.summary.trim().is_empty() {
        report.summary = if report.findings.is_empty() {
            "No issues found.".into()
        } else {
            format!("{} finding(s) require correction.", report.findings.len())
        };
    }
    Ok(report)
}

/// Analysis turns are tool-backed or contain substantial authored prose.
// ponytail: this heuristic avoids reviewing greetings; replace it with explicit
// checkpoints only if real sessions show false positives/negatives.
pub fn should_auto_review(turn: &[Message]) -> bool {
    let has_tool_result = turn.iter().any(|message| {
        message.role == Role::Tool && message.tool_name.as_deref() != Some("attempt_completion")
    });
    let prose_chars: usize = turn
        .iter()
        .filter(|message| {
            message.role == Role::Assistant
                || (message.role == Role::Tool
                    && message.tool_name.as_deref() == Some("attempt_completion"))
        })
        .map(|message| message.content.as_text().chars().count())
        .sum();
    has_tool_result || prose_chars >= 600
}

pub fn correction_prompt(report: &ReviewReport) -> String {
    let findings = report
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
        "An independent reviewer found factual or traceability problems in your latest answer. Correct the answer now. Use tools again if needed, preserve conclusions that remain supported, and explicitly state each correction. Do not discuss the review process itself.\n\n{findings}"
    )
}

pub fn reconcile_follow_up(
    mut original: ReviewReport,
    mut follow_up: ReviewReport,
) -> ReviewReport {
    if follow_up.has_findings() {
        follow_up.id = original.id;
        follow_up.set_status("unaddressed");
        follow_up
    } else {
        original.set_status("resolved");
        original.summary = follow_up.summary;
        original
    }
}

/// Char-boundary-safe truncation (transcripts may be UTF-8 / Chinese).
fn truncate(s: &str, cap: usize) -> String {
    if s.len() <= cap {
        return s.to_string();
    }
    let mut end = cap;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…[truncated {} chars]", &s[..end], s.len() - end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_includes_both_sides_of_a_contradiction() {
        let msgs = vec![
            Message::user("compute x"),
            Message::assistant("The result is x = 5."),
            Message::tool("t1", "python", "print(x) -> 3"),
        ];
        let s = serialize_transcript(&msgs);
        assert!(s.contains("x = 5"), "assistant claim missing:\n{s}");
        assert!(s.contains('3'), "tool value missing:\n{s}");
        assert!(
            s.contains("[msg:2 TOOL:python]"),
            "tool label missing:\n{s}"
        );
        assert!(
            s.contains("[msg:0 USER]") && s.contains("[msg:1 ASSISTANT]"),
            "role labels missing:\n{s}"
        );
    }

    #[test]
    fn serialize_keeps_recent_tail_when_over_budget() {
        let mut msgs = vec![Message::user("OLDEST_MARKER")];
        for _ in 0..40 {
            msgs.push(Message::tool("t", "dump", "y".repeat(5_000)));
        }
        msgs.push(Message::assistant("NEWEST_MARKER"));

        let s = serialize_transcript(&msgs);
        assert!(s.len() <= 90_000, "not capped: {} chars", s.len());
        assert!(s.contains("NEWEST_MARKER"), "recent turn dropped");
        assert!(
            !s.contains("OLDEST_MARKER"),
            "oldest turn should be truncated"
        );
        assert!(
            s.contains("earlier transcript truncated"),
            "missing tail-truncation notice"
        );
    }

    #[test]
    fn parse_report_accepts_fence_and_normalizes_fields() {
        let raw = r#"```json
        {
          "summary": "Checked the reported values.",
          "findings": [
            {
              "message_index": 4,
              "claim": "x is 5",
              "evidence": "the tool printed 3",
              "fix": "state x is 3",
              "verdict": "FAIL",
              "severity": "med"
            },
            {
              "claim": "supported",
              "evidence": "same value",
              "fix": "none",
              "verdict": "pass",
              "severity": "low"
            }
          ]
        }
        ```"#;

        let report = parse_report(raw, "review-model").unwrap();
        assert_eq!(report.reviewer_model, "review-model");
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].verdict, "fail");
        assert_eq!(report.findings[0].severity, "medium");
        assert_eq!(report.findings[0].status, "open");
        assert!(!report.id.is_empty());
    }

    #[test]
    fn auto_review_targets_analysis_not_small_talk() {
        assert!(!should_auto_review(&[Message::assistant("Hello!")]));
        assert!(!should_auto_review(&[Message::tool(
            "t1",
            "attempt_completion",
            "done"
        )]));
        assert!(should_auto_review(&[Message::tool(
            "t1",
            "attempt_completion",
            "x".repeat(600)
        )]));
        assert!(should_auto_review(&[Message::tool("t1", "python", "42")]));
        assert!(should_auto_review(&[Message::assistant("x".repeat(600))]));
    }

    #[test]
    fn correction_prompt_contains_evidence_and_smallest_fix() {
        let report = ReviewReport {
            id: "r1".into(),
            summary: "one problem".into(),
            reviewer_model: "review-model".into(),
            reviewer_effort: String::new(),
            findings: vec![ReviewFinding {
                message_index: 2,
                claim: "x is 5".into(),
                evidence: "tool output is 3".into(),
                fix: "change 5 to 3".into(),
                verdict: "warn".into(),
                severity: "low".into(),
                status: "open".into(),
            }],
        };

        let prompt = correction_prompt(&report);
        assert!(prompt.contains("x is 5"));
        assert!(prompt.contains("tool output is 3"));
        assert!(prompt.contains("change 5 to 3"));
    }

    #[test]
    fn follow_up_resolves_or_preserves_remaining_findings() {
        let original = ReviewReport {
            id: "r1".into(),
            summary: "one problem".into(),
            reviewer_model: "review-model".into(),
            reviewer_effort: String::new(),
            findings: vec![ReviewFinding {
                message_index: 2,
                claim: "x is 5".into(),
                evidence: "tool output is 3".into(),
                fix: "change 5 to 3".into(),
                verdict: "warn".into(),
                severity: "low".into(),
                status: "open".into(),
            }],
        };
        let clean = ReviewReport {
            id: "new-id".into(),
            summary: "Correction verified.".into(),
            reviewer_model: "review-model".into(),
            reviewer_effort: String::new(),
            findings: vec![],
        };
        let resolved = reconcile_follow_up(original.clone(), clean);
        assert_eq!(resolved.id, "r1");
        assert_eq!(resolved.findings[0].status, "resolved");

        let remaining = reconcile_follow_up(original, resolved.clone());
        assert_eq!(remaining.id, "r1");
        assert_eq!(remaining.findings[0].status, "unaddressed");
    }
}
