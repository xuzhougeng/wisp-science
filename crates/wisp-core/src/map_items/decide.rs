//! The decision step: Jev answers yes/no questions about one item's structured
//! row, and a fixed rule turns the calibrated probabilities into a verdict.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wisp_llm::systemone::Question;

/// Written to be conservative about *removing* an item: only `Reject` takes it
/// out of the candidate set, and nothing is ever deleted — verdicts are labels.
/// ponytail: same shape as the desktop autopilot constants; calibrate on a
/// hand-labelled sample before moving them.
pub const DEFAULT_PASS_AT: f64 = 0.8;
pub const DEFAULT_REJECT_AT: f64 = 0.2;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Thresholds {
    pub pass_at: f64,
    pub reject_at: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            pass_at: DEFAULT_PASS_AT,
            reject_at: DEFAULT_REJECT_AT,
        }
    }
}

impl Thresholds {
    pub fn validate(self) -> Result<Self, String> {
        let ok = (0.0..=1.0).contains(&self.reject_at)
            && (0.0..=1.0).contains(&self.pass_at)
            && self.reject_at < self.pass_at;
        ok.then_some(self)
            .ok_or_else(|| "need 0 <= reject_at < pass_at <= 1".into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,
    Reject,
    /// Not decidable from the probabilities: hand back to the caller.
    Uncertain,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Reject => "reject",
            Verdict::Uncertain => "uncertain",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "pass" => Some(Verdict::Pass),
            "reject" => Some(Verdict::Reject),
            "uncertain" => Some(Verdict::Uncertain),
            _ => None,
        }
    }
}

/// Every noul question is phrased so that "yes" means the item meets the
/// requirement. Any answer at or below `reject_at` rejects; every answer at or
/// above `pass_at` passes; everything else — including a missing or NaN
/// probability — is uncertain. `None` when there is no noul question to judge by.
pub fn verdict(
    scores: &BTreeMap<String, f64>,
    questions: &BTreeMap<String, Question>,
    thresholds: Thresholds,
) -> Option<Verdict> {
    let mut judged = false;
    let mut all_pass = true;
    for (id, question) in questions {
        if !matches!(question, Question::Noul { .. }) {
            continue;
        }
        judged = true;
        match scores.get(id).copied().filter(|p| !p.is_nan()) {
            Some(p) if p <= thresholds.reject_at => return Some(Verdict::Reject),
            Some(p) if p >= thresholds.pass_at => {}
            _ => all_pass = false,
        }
    }
    judged.then_some(if all_pass {
        Verdict::Pass
    } else {
        Verdict::Uncertain
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noul(text: &str) -> Question {
        Question::Noul {
            instructions: text.into(),
            criteria: None,
        }
    }

    fn qs(ids: &[&str]) -> BTreeMap<String, Question> {
        ids.iter().map(|id| (id.to_string(), noul(id))).collect()
    }

    fn scores(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn rule_is_conservative_about_rejecting() {
        let t = Thresholds::default();
        let q = qs(&["a", "b"]);
        let v = |s| verdict(&scores(s), &q, t);
        assert_eq!(v(&[("a", 0.95), ("b", 0.8)]), Some(Verdict::Pass));
        assert_eq!(v(&[("a", 0.95), ("b", 0.79)]), Some(Verdict::Uncertain));
        assert_eq!(v(&[("a", 0.95), ("b", 0.2)]), Some(Verdict::Reject));
        // One clear "no" outweighs a clear "yes".
        assert_eq!(v(&[("a", 0.99), ("b", 0.01)]), Some(Verdict::Reject));
        // Missing or NaN is never a pass.
        assert_eq!(v(&[("a", 0.95)]), Some(Verdict::Uncertain));
        assert_eq!(v(&[("a", 0.95), ("b", f64::NAN)]), Some(Verdict::Uncertain));
        assert_eq!(v(&[("a", 0.05), ("b", f64::NAN)]), Some(Verdict::Reject));
    }

    #[test]
    fn only_noul_questions_judge() {
        let mut q = qs(&["a"]);
        q.insert(
            "kind".into(),
            Question::Choice {
                instructions: "x".into(),
                criteria: BTreeMap::new(),
            },
        );
        assert_eq!(
            verdict(&scores(&[("a", 0.9)]), &q, Thresholds::default()),
            Some(Verdict::Pass)
        );
        let only_choice: BTreeMap<_, _> = q.into_iter().filter(|(k, _)| k == "kind").collect();
        assert_eq!(
            verdict(&scores(&[]), &only_choice, Thresholds::default()),
            None
        );
    }

    #[test]
    fn thresholds_must_be_ordered() {
        assert!(Thresholds::default().validate().is_ok());
        assert!(Thresholds {
            pass_at: 0.3,
            reject_at: 0.3
        }
        .validate()
        .is_err());
        assert!(Thresholds {
            pass_at: 1.2,
            reject_at: 0.1
        }
        .validate()
        .is_err());
    }
}
