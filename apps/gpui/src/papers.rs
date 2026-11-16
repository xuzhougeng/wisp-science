//! Paper evidence: how a paper's versions, items and their sources read.
use wisp_server::desktop::{
    Paper, PaperAudience, PaperCheck, PaperItemKind, PaperLevel, PaperSource, PaperSourceChoice,
    PaperSourceKind, PaperVersion, PaperVersionState,
};

use crate::i18n::{fill, tr};

pub fn kind_label(kind: PaperItemKind) -> &'static str {
    match kind {
        PaperItemKind::Figure => tr("Figure"),
        PaperItemKind::Table => tr("Table"),
        PaperItemKind::Claim => tr("Claim"),
        PaperItemKind::Section => tr("Section"),
        PaperItemKind::Methods => tr("Methods"),
        PaperItemKind::Supplement => tr("Supplement"),
    }
}

pub fn state_label(state: PaperVersionState) -> &'static str {
    match state {
        PaperVersionState::Draft => tr("Draft"),
        PaperVersionState::Finalized => tr("Finalized"),
        PaperVersionState::Published => tr("Published"),
        PaperVersionState::Busy => tr("Busy"),
    }
}

/// A version on its button: its number, its name, and its state unless
/// it is a draft like most.
pub fn version_label(version: &PaperVersion) -> String {
    let name = format!("v{} · {}", version.number, version.label);
    match version.state {
        PaperVersionState::Draft => name,
        state => format!("{name} · {}", state_label(state)),
    }
}

pub fn source_kind_label(kind: PaperSourceKind) -> &'static str {
    match kind {
        PaperSourceKind::Result => tr("Result"),
        PaperSourceKind::Run => tr("Run"),
        PaperSourceKind::Other => tr("Other source"),
    }
}

/// A source under its item: what it is, and why it was chosen when that
/// was said.
pub fn source_line(source: &PaperSource) -> String {
    let purpose = source.purpose.trim();
    if purpose.is_empty() {
        source.name.clone()
    } else {
        format!("{} — {purpose}", source.name)
    }
}

/// A result or run in the list to choose from. A result says which of
/// its versions would be bound; a run says how it ended.
pub fn choice_line(choice: &PaperSourceChoice) -> String {
    match (choice.kind, choice.detail.trim()) {
        (_, "") => choice.name.clone(),
        (PaperSourceKind::Result, version) => {
            fill(tr("{} · version {}"), &[&choice.name, &version])
        }
        (_, state) => format!("{} · {state}", choice.name),
    }
}

/// What a paper's shown version holds, in a line.
pub fn summary(paper: &Paper) -> String {
    let sources: usize = paper
        .items
        .iter()
        .map(|item| item.sources.len())
        .sum::<usize>()
        + paper.loose.len();
    let unsupported = paper
        .items
        .iter()
        .filter(|item| item.sources.is_empty())
        .count();
    let mut line = fill(tr("{} items · {} sources"), &[&paper.items.len(), &sources]);
    if unsupported > 0 {
        line.push_str(&fill(tr(" · {} without a source"), &[&unsupported]));
    }
    line
}

pub fn audience_label(audience: PaperAudience) -> &'static str {
    match audience {
        PaperAudience::Private => tr("Private"),
        PaperAudience::Restricted => tr("Restricted"),
        PaperAudience::Public => tr("Public"),
    }
}

pub fn level_label(level: PaperLevel) -> &'static str {
    match level {
        PaperLevel::Archived => tr("Archived"),
        PaperLevel::Traceable => tr("Traceable"),
        PaperLevel::Rerunnable => tr("Re-runnable"),
        PaperLevel::Reproduced => tr("Reproduced"),
    }
}

/// What a new version is called: the number after the paper's last one.
pub fn next_version_label(paper: &Paper) -> String {
    let next = paper
        .versions
        .iter()
        .map(|version| version.number)
        .max()
        .unwrap_or(0)
        + 1;
    fill(tr("Version {}"), &[&next])
}

/// A locked version's fingerprint, short enough to read.
pub fn fingerprint(hash: &str) -> String {
    format!("sha256:{}", hash.get(..12).unwrap_or(hash))
}

/// What the check comes to, in a line: ready, or how much stands in the
/// way.
pub fn verdict(check: &PaperCheck) -> String {
    if check.finalized {
        return tr("Finalized").to_owned();
    }
    let open = check
        .blockers
        .iter()
        .filter(|finding| finding.waived.is_none())
        .count();
    let waived = check.blockers.len() - open;
    match (check.can_finalize, waived) {
        (true, 0) => tr("Ready to finalize").to_owned(),
        (true, waived) => fill(tr("Ready to finalize, with {} let stand"), &[&waived]),
        (false, _) => fill(tr("{} to fix or let stand before finalizing"), &[&open]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_server::desktop::{PaperFinding, PaperItem};

    fn source(name: &str, purpose: &str) -> PaperSource {
        PaperSource {
            id: name.into(),
            kind: PaperSourceKind::Run,
            name: name.into(),
            purpose: purpose.into(),
        }
    }

    fn item(title: &str, sources: Vec<PaperSource>) -> PaperItem {
        PaperItem {
            id: title.into(),
            kind: PaperItemKind::Figure,
            title: title.into(),
            note: String::new(),
            sources,
        }
    }

    #[test]
    fn a_version_names_its_state_only_when_it_is_no_draft() {
        let mut version = PaperVersion {
            id: "v".into(),
            number: 2,
            label: "Revision".into(),
            state: PaperVersionState::Draft,
            fingerprint: String::new(),
        };
        assert_eq!(version_label(&version), "v2 · Revision");
        version.state = PaperVersionState::Finalized;
        assert_eq!(version_label(&version), "v2 · Revision · Finalized");
    }

    #[test]
    fn sources_and_choices_say_what_they_are() {
        assert_eq!(source_line(&source("fit.py", " ")), "fit.py");
        assert_eq!(
            source_line(&source("fit.py", "panel B")),
            "fit.py — panel B"
        );
        let mut choice = PaperSourceChoice {
            kind: PaperSourceKind::Result,
            id: "a".into(),
            name: "fig1.png".into(),
            detail: "3".into(),
        };
        assert_eq!(choice_line(&choice), "fig1.png · version 3");
        choice.kind = PaperSourceKind::Run;
        choice.detail = "succeeded".into();
        assert_eq!(choice_line(&choice), "fig1.png · succeeded");
        choice.detail.clear();
        assert_eq!(choice_line(&choice), "fig1.png");
    }

    #[test]
    fn the_summary_counts_what_still_has_no_source() {
        let mut paper = Paper {
            id: "p".into(),
            title: "Paper".into(),
            description: String::new(),
            versions: vec![],
            shown: "v".into(),
            editable: true,
            items: vec![
                item("Figure 1", vec![source("a", ""), source("b", "")]),
                item("Figure 2", vec![]),
            ],
            loose: vec![source("c", "")],
        };
        assert_eq!(summary(&paper), "2 items · 3 sources · 1 without a source");
        paper.items.pop();
        assert_eq!(summary(&paper), "1 items · 3 sources");
    }

    fn found(code: &str, waived: Option<&str>) -> PaperFinding {
        PaperFinding {
            code: code.into(),
            message: code.into(),
            waivable: true,
            waived: waived.map(str::to_owned),
        }
    }

    #[test]
    fn the_verdict_counts_what_stands_in_the_way() {
        let mut check = PaperCheck {
            finalized: false,
            level: PaperLevel::Traceable,
            blockers: vec![found("run_code_missing", None), found("a", Some("kept"))],
            warnings: vec![],
            omissions: vec![],
            fingerprint: "ab".repeat(32),
            can_finalize: false,
        };
        assert_eq!(verdict(&check), "1 to fix or let stand before finalizing");
        check.blockers[0].waived = Some("by hand".into());
        check.can_finalize = true;
        assert_eq!(verdict(&check), "Ready to finalize, with 2 let stand");
        check.blockers.clear();
        assert_eq!(verdict(&check), "Ready to finalize");
        check.finalized = true;
        assert_eq!(verdict(&check), "Finalized");
        assert_eq!(fingerprint(&check.fingerprint), "sha256:abababababab");
        assert_eq!(fingerprint("abc"), "sha256:abc");
    }

    #[test]
    fn a_new_version_is_numbered_after_the_last() {
        let version = |number| PaperVersion {
            id: format!("v{number}"),
            number,
            label: String::new(),
            state: PaperVersionState::Finalized,
            fingerprint: String::new(),
        };
        let mut paper = Paper {
            id: "p".into(),
            title: "Paper".into(),
            description: String::new(),
            versions: vec![version(3), version(1)],
            shown: "v3".into(),
            editable: false,
            items: vec![],
            loose: vec![],
        };
        assert_eq!(next_version_label(&paper), "Version 4");
        paper.versions.clear();
        assert_eq!(next_version_label(&paper), "Version 1");
    }
}
