//! L1 session review: serialize a frame's transcript into plain text so a
//! one-shot reviewer LLM call can trace it. No sub-agent, no tools — the
//! reviewer only reads what we hand it here.

use wisp_llm::{Message, Role};

/// Reviewer system prompt. Self-authored (an Apache-2.0 repo must not bundle
/// the upstream proprietary REVIEWER prompt) — captures the same job: trace the
/// transcript, don't recompute; a finding needs transcript evidence.
pub const REVIEWER_RUBRIC: &str = "\
You are a REVIEWER. You are given a transcript of another agent's working \
session — user turns, the agent's replies, and tool outputs (`[TOOL:name]`). \
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
- Do not restate what the agent did correctly. Only report problems.

Output GitHub-flavored Markdown, nothing else:
- If you find no problems: `✓ No issues found.` and one line on what you traced.
- Otherwise, one block per finding:

### [HIGH|MED|LOW] <fabrication | hallucination | plan-deviation>
- **Claim:** <what the agent asserted>
- **Evidence:** <the transcript/tool line that contradicts it>
- **Fix:** <the smallest correction>

Order findings most severe first.";

/// Per-tool-result char cap: tool dumps (CSVs, stack traces) are the p90 size
/// driver; the reviewer traces claims, it doesn't need the full dump.
const PER_TOOL_CAP: usize = 4_000;
/// Whole-transcript char cap, kept from the tail (most recent turns).
// ponytail: char-based tail window; a huge session is reviewed from its recent
// end only. Upgrade path: per-checkpoint windowing if long-session recall matters.
const TOTAL_CAP: usize = 80_000;

/// Render a message list as `[USER]/[ASSISTANT]/[TOOL:name]` blocks. System
/// messages (wisp's own instructions) are dropped — they're not under review.
pub fn serialize_transcript(msgs: &[Message]) -> String {
    let mut blocks: Vec<String> = Vec::new();
    for m in msgs {
        let block = match m.role {
            Role::System => continue,
            Role::User => format!("[USER]\n{}", m.content.as_text()),
            Role::Assistant => {
                let mut b = format!("[ASSISTANT]\n{}", m.content.as_text());
                for tc in &m.tool_calls {
                    b.push_str(&format!("\n[CALL {}] {}", tc.function.name, tc.function.arguments));
                }
                b
            }
            Role::Tool => {
                let name = m.tool_name.as_deref().unwrap_or("tool");
                format!("[TOOL:{}]\n{}", name, truncate(&m.content.as_text(), PER_TOOL_CAP))
            }
        };
        blocks.push(block);
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
        assert!(s.contains("[TOOL:python]"), "tool label missing:\n{s}");
        assert!(
            s.contains("[USER]") && s.contains("[ASSISTANT]"),
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
        assert!(!s.contains("OLDEST_MARKER"), "oldest turn should be truncated");
        assert!(
            s.contains("earlier transcript truncated"),
            "missing tail-truncation notice"
        );
    }
}
