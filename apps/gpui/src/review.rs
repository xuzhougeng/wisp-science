//! A review of the open conversation, as its dialog words it. The host
//! asks the model and reads the report; this only says what it found.
use crate::i18n::tr;
use wisp_server::desktop::{Review, ReviewFinding, ReviewOutcome};

use crate::present::Tone;

/// Where a review stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asked {
    /// The reviewer is reading the conversation.
    Reading,
    /// There is no report, and why.
    Failed(String),
    Done(Review),
}

/// The line above the composer about the review that followed the latest
/// turn, and whether there is a report to open.
pub fn banner(outcome: &ReviewOutcome) -> (String, Tone, bool) {
    match outcome {
        ReviewOutcome::Reading => (tr("Reviewing the answer…").into(), Tone::Neutral, false),
        ReviewOutcome::Failed(problem) => (
            format!("The answer was not reviewed: {problem}"),
            Tone::Attention,
            false,
        ),
        ReviewOutcome::Done(found) => {
            let (said, tone) = outcome_of(found);
            (format!("Review: {said}"), tone, !found.findings.is_empty())
        }
    }
}

/// The line under the title once the report is in.
pub fn outcome(review: &Review) -> (String, Tone) {
    outcome_of(review)
}

fn outcome_of(review: &Review) -> (String, Tone) {
    let failed = review
        .findings
        .iter()
        .filter(|finding| finding.verdict == "fail")
        .count();
    let warned = review.findings.len() - failed;
    match (failed, warned) {
        (0, 0) => (
            tr("No problems found in what was checked.").into(),
            Tone::Success,
        ),
        (0, warned) => (
            format!("{} to look at.", count(warned, "thing")),
            Tone::Attention,
        ),
        (failed, 0) => (
            format!("{} to correct.", count(failed, "problem")),
            Tone::Danger,
        ),
        (failed, warned) => (
            format!(
                "{} to correct, {} to look at.",
                count(failed, "problem"),
                count(warned, "thing")
            ),
            Tone::Danger,
        ),
    }
}

fn count(number: usize, noun: &str) -> String {
    match number {
        1 => format!("1 {noun}"),
        _ => format!("{number} {noun}s"),
    }
}

/// A finding's heading: how sure the reviewer is and how much it matters.
pub fn finding_heading(finding: &ReviewFinding) -> (String, Tone) {
    let (kind, tone) = match finding.verdict.as_str() {
        "fail" => (tr("Wrong"), Tone::Danger),
        _ => (tr("Doubtful"), Tone::Attention),
    };
    let weight = match finding.severity.as_str() {
        "high" => "matters a lot",
        "medium" => "matters",
        _ => "minor",
    };
    (format!("{kind} · {weight}"), tone)
}

/// The composer's text once the correction is put in it: after whatever
/// was already being written, which is kept, and never twice.
pub fn with_correction(typed: &str, review: &Review) -> String {
    let correction = review.correction();
    let typed = typed.trim();
    if typed.is_empty() || typed.ends_with(&correction) {
        return if typed.is_empty() {
            correction
        } else {
            typed.to_owned()
        };
    }
    format!("{typed}\n\n{correction}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(verdict: &str, severity: &str) -> ReviewFinding {
        ReviewFinding {
            claim: "100 reads".into(),
            evidence: "wc printed 400".into(),
            fix: "Say 400 lines".into(),
            verdict: verdict.into(),
            severity: severity.into(),
        }
    }

    fn review(findings: Vec<ReviewFinding>) -> Review {
        Review {
            summary: "Checked the counts.".into(),
            findings,
            model: "m-1".into(),
        }
    }

    #[test]
    fn the_outcome_counts_what_is_wrong_and_what_is_doubtful() {
        assert_eq!(
            outcome(&review(vec![])),
            (
                "No problems found in what was checked.".into(),
                Tone::Success
            )
        );
        assert_eq!(
            outcome(&review(vec![finding("warn", "low")])),
            ("1 thing to look at.".into(), Tone::Attention)
        );
        assert_eq!(
            outcome(&review(vec![
                finding("fail", "high"),
                finding("fail", "low")
            ])),
            ("2 problems to correct.".into(), Tone::Danger)
        );
        assert_eq!(
            outcome(&review(vec![
                finding("fail", "high"),
                finding("warn", "low"),
                finding("warn", "low")
            ])),
            (
                "1 problem to correct, 2 things to look at.".into(),
                Tone::Danger
            )
        );
    }

    #[test]
    fn the_banner_says_where_the_review_of_the_latest_turn_stands() {
        assert_eq!(
            banner(&ReviewOutcome::Reading),
            ("Reviewing the answer…".into(), Tone::Neutral, false)
        );
        assert_eq!(
            banner(&ReviewOutcome::Failed("no report".into())),
            (
                "The answer was not reviewed: no report".into(),
                Tone::Attention,
                false
            )
        );
        // Nothing found: said, with nothing to open.
        assert_eq!(
            banner(&ReviewOutcome::Done(review(vec![]))),
            (
                "Review: No problems found in what was checked.".into(),
                Tone::Success,
                false
            )
        );
        assert_eq!(
            banner(&ReviewOutcome::Done(review(vec![finding("fail", "high")]))),
            ("Review: 1 problem to correct.".into(), Tone::Danger, true)
        );
    }

    #[test]
    fn a_finding_is_headed_by_how_sure_and_how_much() {
        assert_eq!(
            finding_heading(&finding("fail", "high")),
            ("Wrong · matters a lot".into(), Tone::Danger)
        );
        assert_eq!(
            finding_heading(&finding("warn", "medium")),
            ("Doubtful · matters".into(), Tone::Attention)
        );
        // Anything else the reviewer wrote reads as the mildest.
        assert_eq!(
            finding_heading(&finding("", "")),
            ("Doubtful · minor".into(), Tone::Attention)
        );
    }

    #[test]
    fn the_correction_follows_what_was_typed_and_is_added_once() {
        let found = review(vec![finding("fail", "high")]);
        let alone = with_correction("  ", &found);
        assert_eq!(alone, found.correction());
        let after = with_correction("Keep the table. ", &found);
        assert_eq!(after, format!("Keep the table.\n\n{}", found.correction()));
        assert_eq!(with_correction(&after, &found), after);
        assert_eq!(with_correction(&alone, &found), alone);
    }
}
