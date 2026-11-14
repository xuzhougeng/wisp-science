//! Paper evidence: how a paper's versions, items and their sources read.
use wisp_server::desktop::{
    Paper, PaperItemKind, PaperSource, PaperSourceChoice, PaperSourceKind, PaperVersion,
    PaperVersionState,
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

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_server::desktop::PaperItem;

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
}
