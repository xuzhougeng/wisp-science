//! Paper evidence: how a paper's versions, items and their sources read,
//! what is being set as evidence from elsewhere, where a source came from,
//! and how clean reruns and capsules of a finalized version read.
use wisp_server::desktop::{
    Paper, PaperAudience, PaperCapsule, PaperCapsuleStatus, PaperCapture, PaperCheck, PaperDrift,
    PaperItem, PaperItemKind, PaperLevel, PaperLineage, PaperLineageQuality, PaperRerun,
    PaperRerunOutput, PaperRerunStatus, PaperSelection, PaperSource, PaperSourceKind, PaperVersion,
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
        PaperSourceKind::Passage => tr("Passage"),
        PaperSourceKind::ToolCall => tr("Tool result"),
        PaperSourceKind::ExecutionLog => tr("Execution log"),
        PaperSourceKind::CodeCell => tr("Code cell"),
        PaperSourceKind::Resource => tr("External resource"),
    }
}

pub fn selection_label(selection: PaperSelection) -> &'static str {
    match selection {
        PaperSelection::Candidate => tr("Candidate"),
        PaperSelection::Selected => tr("Selected"),
        PaperSelection::Rejected => tr("Rejected"),
    }
}

/// The items `editing` may be put under: every other item of the shown
/// version except those inside it, which would put it inside itself. A new
/// item (`editing` empty) may go under any.
pub fn parent_choices<'a>(paper: &'a Paper, editing: &str) -> Vec<&'a PaperItem> {
    let mut inside = std::collections::HashSet::new();
    if !editing.is_empty() {
        inside.insert(editing);
        // Items come in outline order, so a child follows its parent.
        for item in &paper.items {
            if item
                .parent
                .as_deref()
                .is_some_and(|parent| inside.contains(parent))
            {
                inside.insert(item.id.as_str());
            }
        }
    }
    paper
        .items
        .iter()
        .filter(|item| !inside.contains(item.id.as_str()))
        .collect()
}

pub fn quality_label(quality: PaperLineageQuality) -> &'static str {
    match quality {
        PaperLineageQuality::Exact => tr("Exact"),
        PaperLineageQuality::Likely => tr("Likely"),
        PaperLineageQuality::Uncertain => tr("Uncertain"),
    }
}

/// The analysis chain under a source, under the WebView card's headings:
/// the files the run read, the run with its code, and the source itself.
pub fn lineage_chain(source: &PaperSource) -> [(&'static str, String); 3] {
    let lineage = &source.lineage;
    let unrecorded = || tr("Source not recorded").to_owned();
    let mut run = lineage.run_title.clone().unwrap_or_else(unrecorded);
    if !lineage.code.is_empty() {
        run.push_str(" · ");
        run.push_str(&lineage.code.join(", "));
    }
    [
        (
            tr("Project results"),
            if lineage.inputs.is_empty() {
                unrecorded()
            } else {
                lineage.inputs.join(", ")
            },
        ),
        (tr("Analysis runs"), run),
        (tr("Selected result"), source.name.clone()),
    ]
}

/// The source and environment details, one fact a line, as the WebView's
/// collapsed details list them.
pub fn lineage_details(lineage: &PaperLineage) -> Vec<String> {
    let mut lines = vec![fill(tr("Lineage: {}"), &[&quality_label(lineage.quality)])];
    if let Some(version) = lineage.version {
        lines.push(format!("v{version}"));
    }
    if let Some(checksum) = &lineage.checksum {
        lines.push(fingerprint(checksum));
    }
    if let Some(capture) = lineage.capture {
        lines.push(
            match capture {
                PaperCapture::AtCreation => tr("Captured at creation"),
                PaperCapture::Late => tr("Late capture"),
                PaperCapture::Unknown => tr("Capture time unknown"),
            }
            .to_owned(),
        );
    }
    lines.push(fill(
        tr("{} inputs · {} outputs · {} code snapshots"),
        &[
            &lineage.input_count,
            &lineage.output_count,
            &lineage.code_count,
        ],
    ));
    lines.push(
        if lineage.environment {
            tr("Environment captured")
        } else {
            tr("Environment missing")
        }
        .to_owned(),
    );
    if !lineage.bases.is_empty() {
        lines.push(lineage.bases.join(", "));
    }
    if let Some(run) = &lineage.run_id {
        lines.push(format!("run:{run}"));
    }
    lines
}

/// The note on a result the project has a newer version of.
pub fn drift_line(drift: PaperDrift) -> String {
    fill(
        tr("A newer result exists: bound {}; latest {}"),
        &[&format!("v{}", drift.bound), &format!("v{}", drift.latest)],
    )
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

/// A result or a run being set as evidence from where it was found (a
/// file under Artifacts), until the item it is for is chosen on the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingEvidence {
    pub kind: PaperSourceKind,
    /// The artifact (bound at the version it has now) or the run.
    pub id: String,
    pub name: String,
}

/// Why the evidence being set cannot go on an item yet, if it cannot: no
/// paper is shown, the shown version is finalized, or it has no item.
pub fn pending_note(paper: Option<&Paper>) -> Option<&'static str> {
    match paper {
        None => Some(tr(
            "Start a paper first, then add the figure or table it is for.",
        )),
        Some(paper) if !paper.editable => Some(tr(
            "The shown version is finalized. Start a new version to set evidence.",
        )),
        Some(paper) if paper.items.is_empty() => Some(tr(
            "Add the figure, table or section it is for, then choose “Use here”.",
        )),
        Some(_) => None,
    }
}

pub fn rerun_status_label(status: PaperRerunStatus) -> &'static str {
    match status {
        PaperRerunStatus::Running => tr("Running"),
        PaperRerunStatus::Completed => tr("Completed"),
        PaperRerunStatus::Failed => tr("Failed"),
    }
}

/// A clean rerun in a line: how it ended, what it showed, whether the
/// environment was the recorded one, and its exit code.
pub fn rerun_line(rerun: &PaperRerun) -> String {
    let mut line = format!(
        "{} · {} · {}",
        rerun_status_label(rerun.status),
        level_label(rerun.level),
        if rerun.environment_matched {
            tr("Environment matched")
        } else {
            tr("Environment mismatch")
        }
    );
    if let Some(code) = rerun.exit_code {
        line.push_str(&fill(tr(" · exit {}"), &[&code]));
    }
    line
}

/// One compared output: its path and how it was compared.
pub fn rerun_output_line(output: &PaperRerunOutput) -> String {
    format!("{} · {}", output.path, output.comparator)
}

pub fn capsule_status_label(status: PaperCapsuleStatus) -> &'static str {
    match status {
        PaperCapsuleStatus::Building => tr("Building"),
        PaperCapsuleStatus::Succeeded => tr("Succeeded"),
        PaperCapsuleStatus::Failed => tr("Failed"),
    }
}

/// A capsule build in a line: who it is for and, once written, the
/// archive's fingerprint.
pub fn capsule_line(capsule: &PaperCapsule) -> String {
    let mut line = format!(
        "{} · zip · {}",
        capsule_status_label(capsule.status),
        audience_label(capsule.audience)
    );
    if let Some(archive) = &capsule.archive {
        line.push_str(" · ");
        line.push_str(&fingerprint(archive));
    }
    line
}

/// Whether the shown version can be rerun and saved as a capsule: it is
/// finalized or published.
pub fn shown_is_finalized(paper: &Paper) -> bool {
    paper.versions.iter().any(|version| {
        version.id == paper.shown
            && matches!(
                version.state,
                PaperVersionState::Finalized | PaperVersionState::Published
            )
    })
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
    use wisp_server::desktop::PaperFinding;

    fn lineage() -> PaperLineage {
        PaperLineage {
            inputs: vec![],
            run_title: None,
            run_id: None,
            code: vec![],
            quality: PaperLineageQuality::Uncertain,
            version: None,
            checksum: None,
            capture: None,
            input_count: 0,
            output_count: 0,
            code_count: 0,
            environment: false,
            bases: vec![],
        }
    }

    fn source(name: &str, purpose: &str) -> PaperSource {
        PaperSource {
            id: name.into(),
            kind: PaperSourceKind::Run,
            name: name.into(),
            purpose: purpose.into(),
            run: Some(name.into()),
            selection: PaperSelection::Selected,
            audience: PaperAudience::Private,
            lineage: lineage(),
            drift: None,
        }
    }

    fn item(title: &str, sources: Vec<PaperSource>) -> PaperItem {
        PaperItem {
            id: title.into(),
            kind: PaperItemKind::Figure,
            title: title.into(),
            note: String::new(),
            parent: None,
            depth: 0,
            sources,
        }
    }

    #[test]
    fn an_item_is_not_offered_a_place_inside_itself() {
        let mut paper = Paper {
            id: "p".into(),
            title: "Paper".into(),
            description: String::new(),
            versions: vec![],
            shown: "v".into(),
            editable: true,
            items: vec![
                item("Results", vec![]),
                item("Figure 1", vec![]),
                item("Panel A", vec![]),
                item("Methods", vec![]),
            ],
            loose: vec![],
            level: None,
            reruns: vec![],
            capsules: vec![],
        };
        paper.items[1].parent = Some("Results".into());
        paper.items[2].parent = Some("Figure 1".into());
        let offered = |editing: &str| -> Vec<String> {
            parent_choices(&paper, editing)
                .into_iter()
                .map(|item| item.id.clone())
                .collect()
        };
        assert_eq!(offered("Results"), ["Methods"]);
        assert_eq!(offered("Figure 1"), ["Results", "Methods"]);
        assert_eq!(offered("").len(), 4);
    }

    #[test]
    fn a_source_says_where_it_came_from_and_what_is_newer() {
        let mut found = source("figure1.png", "");
        assert_eq!(
            lineage_chain(&found).map(|(_, value)| value),
            [
                "Source not recorded".to_owned(),
                "Source not recorded".to_owned(),
                "figure1.png".to_owned()
            ]
        );
        found.lineage = PaperLineage {
            inputs: vec!["data/a.csv".into(), "data/b.csv".into()],
            run_title: Some("fit".into()),
            run_id: Some("run-1".into()),
            code: vec!["fit.py".into()],
            quality: PaperLineageQuality::Exact,
            version: Some(2),
            checksum: Some("ab".repeat(32)),
            capture: Some(PaperCapture::Late),
            input_count: 2,
            output_count: 1,
            code_count: 1,
            environment: true,
            bases: vec!["declared".into()],
        };
        assert_eq!(
            lineage_chain(&found).map(|(_, value)| value),
            [
                "data/a.csv, data/b.csv".to_owned(),
                "fit · fit.py".to_owned(),
                "figure1.png".to_owned()
            ]
        );
        assert_eq!(
            lineage_details(&found.lineage),
            [
                "Lineage: Exact",
                "v2",
                "sha256:abababababab",
                "Late capture",
                "2 inputs · 1 outputs · 1 code snapshots",
                "Environment captured",
                "declared",
                "run:run-1",
            ]
        );
        assert_eq!(
            lineage_details(&lineage()),
            [
                "Lineage: Uncertain",
                "0 inputs · 0 outputs · 0 code snapshots",
                "Environment missing",
            ]
        );
        assert_eq!(
            drift_line(PaperDrift {
                bound: 1,
                latest: 3
            }),
            "A newer result exists: bound v1; latest v3"
        );
        assert_eq!(selection_label(PaperSelection::Candidate), "Candidate");
        assert_eq!(source_kind_label(PaperSourceKind::ToolCall), "Tool result");
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
    fn a_source_says_what_it_is_and_why() {
        assert_eq!(source_line(&source("fit.py", " ")), "fit.py");
        assert_eq!(
            source_line(&source("fit.py", "panel B")),
            "fit.py — panel B"
        );
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
            level: None,
            reruns: vec![],
            capsules: vec![],
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
            level: Some(PaperLevel::Rerunnable),
            reruns: vec![],
            capsules: vec![],
        };
        assert_eq!(next_version_label(&paper), "Version 4");
        paper.versions.clear();
        assert_eq!(next_version_label(&paper), "Version 1");
    }

    #[test]
    fn evidence_being_set_says_what_stands_in_its_way() {
        assert!(pending_note(None).unwrap().contains("Start a paper"));
        let mut paper = Paper {
            id: "p".into(),
            title: "Paper".into(),
            description: String::new(),
            versions: vec![],
            shown: "v".into(),
            editable: false,
            items: vec![],
            loose: vec![],
            level: None,
            reruns: vec![],
            capsules: vec![],
        };
        assert!(pending_note(Some(&paper)).unwrap().contains("finalized"));
        paper.editable = true;
        assert!(pending_note(Some(&paper)).unwrap().contains("Use here"));
        paper.items.push(item("Figure 1", vec![]));
        assert_eq!(pending_note(Some(&paper)), None);
    }

    #[test]
    fn reruns_and_capsules_say_how_they_went() {
        let mut rerun = PaperRerun {
            id: "r".into(),
            run: "run".into(),
            status: PaperRerunStatus::Completed,
            level: PaperLevel::Reproduced,
            environment_matched: true,
            exit_code: Some(0),
            error: None,
            outputs: vec![PaperRerunOutput {
                path: "results/out.txt".into(),
                comparator: "sha256".into(),
                passed: true,
            }],
        };
        assert_eq!(
            rerun_line(&rerun),
            "Completed · Reproduced · Environment matched · exit 0"
        );
        rerun.environment_matched = false;
        rerun.exit_code = None;
        rerun.status = PaperRerunStatus::Failed;
        rerun.level = PaperLevel::Rerunnable;
        assert_eq!(
            rerun_line(&rerun),
            "Failed · Re-runnable · Environment mismatch"
        );
        assert_eq!(
            rerun_output_line(&rerun.outputs[0]),
            "results/out.txt · sha256"
        );
        let mut capsule = PaperCapsule {
            id: "c".into(),
            status: PaperCapsuleStatus::Succeeded,
            audience: PaperAudience::Private,
            path: "/tmp/a.zip".into(),
            archive: Some("cd".repeat(32)),
            error: None,
        };
        assert_eq!(
            capsule_line(&capsule),
            "Succeeded · zip · Private · sha256:cdcdcdcdcdcd"
        );
        capsule.status = PaperCapsuleStatus::Failed;
        capsule.archive = None;
        assert_eq!(capsule_line(&capsule), "Failed · zip · Private");
    }

    #[test]
    fn only_a_finalized_or_published_version_is_rerun_or_saved() {
        let version = |id: &str, state| PaperVersion {
            id: id.into(),
            number: 1,
            label: String::new(),
            state,
            fingerprint: String::new(),
        };
        let mut paper = Paper {
            id: "p".into(),
            title: "Paper".into(),
            description: String::new(),
            versions: vec![
                version("a", PaperVersionState::Finalized),
                version("b", PaperVersionState::Draft),
                version("c", PaperVersionState::Published),
                version("d", PaperVersionState::Busy),
            ],
            shown: "a".into(),
            editable: false,
            items: vec![],
            loose: vec![],
            level: None,
            reruns: vec![],
            capsules: vec![],
        };
        assert!(shown_is_finalized(&paper));
        for (shown, finalized) in [("b", false), ("c", true), ("d", false), ("", false)] {
            paper.shown = shown.into();
            assert_eq!(shown_is_finalized(&paper), finalized, "{shown}");
        }
    }
}
