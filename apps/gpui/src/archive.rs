//! The research archive's draft as the researcher reviews it, as the
//! WebView's review has it: a choice for each local file the conversation
//! recorded (keep a snapshot, keep it in place, or delete it for good), the
//! scripts to read and change, and the confirmation that the review was
//! read. What is frozen is what [`Review::request`] carries. No graphics
//! state.
use crate::i18n::{fill, tr};
use wisp_server::desktop::{
    ArchiveFile, ArchiveFileChoice, ArchiveScript, ConfirmResearchArchive, ResearchArchive,
};

/// What freezing does with one listed file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAction {
    /// A copy of the reviewed bytes goes into the archive; the file stays.
    Snapshot,
    /// The file stays where it is, recorded by its checksum.
    Reference,
    /// The file is deleted for good once the archive is frozen.
    Delete,
}

impl FileAction {
    /// In the order the WebView offers them.
    pub const ALL: [Self; 3] = [Self::Snapshot, Self::Reference, Self::Delete];

    /// From the host's word for it; anything else is the default snapshot.
    pub fn of(action: &str) -> Self {
        match action {
            "reference" => Self::Reference,
            "delete" => Self::Delete,
            _ => Self::Snapshot,
        }
    }

    /// The host's word for it.
    pub fn key(self) -> &'static str {
        match self {
            Self::Snapshot => "snapshot",
            Self::Reference => "reference",
            Self::Delete => "delete",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Snapshot => tr("Keep snapshot"),
            Self::Reference => tr("Keep in place"),
            Self::Delete => tr("Permanently delete"),
        }
    }
}

/// Why the dialog asks before the draft's changes are given up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leaving {
    /// The dialog is being closed.
    Close,
    /// The model is to write the draft again.
    Redraft,
}

/// A kept draft under review.
#[derive(Clone, Debug, PartialEq)]
pub struct Review {
    /// The draft as the host kept it.
    pub draft: ResearchArchive,
    /// The title and report as they were put in their fields.
    base_title: String,
    base_report: String,
    /// The scripts as changed so far, and as the editor first showed
    /// them; only freezing writes them.
    pub scripts: Vec<ArchiveScript>,
    base_scripts: Vec<ArchiveScript>,
    /// The choice for each of the draft's files, in its order, and as the
    /// draft made them.
    pub actions: Vec<FileAction>,
    base_actions: Vec<FileAction>,
    /// The script open in the editor.
    pub shown: Option<usize>,
    /// The review the researcher said they read, as it stood then: any
    /// change after takes the confirmation back.
    accepted: Option<String>,
    /// The freeze is under way.
    pub freezing: bool,
    /// The question before the changes are given up, while it is asked.
    pub asking: Option<Leaving>,
}

impl Review {
    /// `title` and `report` are what the dialog put in its fields.
    pub fn new(draft: ResearchArchive, title: String, report: String) -> Self {
        // As the code editor holds them: its line endings are plain
        // newlines, so opening a script is not a change to it.
        let scripts: Vec<_> = draft
            .scripts
            .iter()
            .map(|script| ArchiveScript {
                filename: script.filename.clone(),
                content: crate::files::editor_text(&script.content),
            })
            .collect();
        let actions: Vec<_> = draft
            .files
            .iter()
            .map(|file| match FileAction::of(&file.action) {
                // The host never offers this; the review does not either.
                FileAction::Delete if !file.can_delete => FileAction::Snapshot,
                action => action,
            })
            .collect();
        Self {
            shown: (!scripts.is_empty()).then_some(0),
            base_scripts: scripts.clone(),
            scripts,
            draft,
            base_title: title,
            base_report: report,
            base_actions: actions.clone(),
            actions,
            accepted: None,
            freezing: false,
            asking: None,
        }
    }

    /// Chooses `action` for the file at `index`. Deleting is refused for a
    /// file that may not be deleted: an input, one used elsewhere, or one
    /// of uncertain ownership.
    pub fn choose(&mut self, index: usize, action: FileAction) -> bool {
        let Some(file) = self.draft.files.get(index) else {
            return false;
        };
        if action == FileAction::Delete && !file.can_delete {
            return false;
        }
        self.actions[index] = action;
        true
    }

    /// The files chosen for deletion.
    pub fn deletions(&self) -> Vec<&ArchiveFile> {
        self.draft
            .files
            .iter()
            .zip(&self.actions)
            .filter(|(_, action)| **action == FileAction::Delete)
            .map(|(file, _)| file)
            .collect()
    }

    /// How many files freezing deletes for good, and their size.
    pub fn deletion_line(&self) -> String {
        let deleted = self.deletions();
        if deleted.is_empty() {
            return tr("No file is selected for permanent deletion.").into();
        }
        let bytes: u64 = deleted.iter().map(|file| file.size_bytes).sum();
        fill(
            tr("Selected for permanent deletion: {} ({})"),
            &[
                &crate::files::count(deleted.len()),
                &crate::files::size_label(bytes),
            ],
        )
    }

    /// What is frozen: the title and report from their fields, the
    /// scripts as changed and a choice for every listed file.
    pub fn request(&self, title: &str, report: &str) -> ConfirmResearchArchive {
        ConfirmResearchArchive {
            id: self.draft.id.clone(),
            title: title.trim().to_owned(),
            report: report.to_owned(),
            scripts: self.scripts.clone(),
            files: self
                .draft
                .files
                .iter()
                .zip(&self.actions)
                .map(|(file, action)| ArchiveFileChoice {
                    path: file.path.clone(),
                    action: action.key().to_owned(),
                })
                .collect(),
        }
    }

    /// Whether anything was changed since the draft was opened.
    pub fn edited(&self, title: &str, report: &str) -> bool {
        title != self.base_title
            || report != self.base_report
            || self.scripts != self.base_scripts
            || self.actions != self.base_actions
    }

    /// Why it cannot be frozen as it is, by the WebView's checks.
    pub fn problem(&self, title: &str, report: &str) -> Option<String> {
        let request = self.request(title, report);
        wisp_server::desktop::research_archive_problem(
            &request.title,
            &request.report,
            &request.scripts,
        )
        .err()
        .map(|problem| crate::i18n::host(&problem))
    }

    fn fingerprint(&self, title: &str, report: &str) -> String {
        serde_json::to_string(&self.request(title, report)).unwrap_or_default()
    }

    /// Says the review as it stands was read, or takes that back.
    pub fn accept(&mut self, title: &str, report: &str, read: bool) {
        self.accepted = read.then(|| self.fingerprint(title, report));
    }

    /// Whether the review as it stands now was said to be read.
    pub fn accepted(&self, title: &str, report: &str) -> bool {
        self.accepted.as_deref() == Some(self.fingerprint(title, report).as_str())
    }

    /// Whether it can be frozen now.
    pub fn ready(&self, title: &str, report: &str) -> bool {
        !self.freezing && self.accepted(title, report) && self.problem(title, report).is_none()
    }

    /// Keeps what the script editor holds as the shown script.
    pub fn note_script(&mut self, filename: &str, content: &str) {
        if let Some(script) = self.shown.and_then(|at| self.scripts.get_mut(at)) {
            script.filename = filename.to_owned();
            script.content = content.to_owned();
        }
    }

    /// Opens the script at `index`; the one shown before keeps its changes.
    pub fn show(&mut self, index: usize) -> Option<&ArchiveScript> {
        let script = self.scripts.get(index)?;
        self.shown = Some(index);
        Some(script)
    }
}

/// The highlighting for a script, by its name.
pub fn highlighter(filename: &str) -> &'static str {
    let kind = filename
        .rsplit_once('.')
        .map(|(_, kind)| kind.to_ascii_lowercase())
        .unwrap_or_default();
    match kind.as_str() {
        "py" | "pyw" => "python",
        "r" => "r",
        "sh" | "bash" => "bash",
        "js" | "mjs" => "javascript",
        "json" => "json",
        "md" => "markdown",
        _ => "text",
    }
}

/// The part of a host's bilingual note ("English / 中文") in the language
/// in use; a note that is not bilingual as it is.
pub fn bilingual(note: &str, chinese: bool) -> &str {
    match note.split_once(" / ") {
        Some((english, chinese_part)) => {
            if chinese {
                chinese_part
            } else {
                english
            }
        }
        None => note,
    }
}

/// What became of a file a frozen archive chose to delete.
pub fn cleanup_line(status: &str) -> Option<String> {
    match status {
        "" => None,
        "deleted" => Some(tr("Deleted").into()),
        "pending" => Some(tr("Not deleted yet").into()),
        other => Some(match other.strip_prefix("not deleted: ") {
            Some(why) => fill(tr("Not deleted: {}"), &[&why]),
            None => other.to_owned(),
        }),
    }
}

/// Whether a frozen archive has files it chose to delete that are not.
pub fn cleanup_incomplete(archive: &ResearchArchive) -> bool {
    archive
        .files
        .iter()
        .any(|file| file.action == "delete" && file.cleanup_status != "deleted")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, action: &str, can_delete: bool, size_bytes: u64) -> ArchiveFile {
        ArchiveFile {
            path: path.into(),
            checksum: "c".into(),
            size_bytes,
            action: action.into(),
            can_delete,
            reason: "Exclusive recorded creation / 本会话创建".into(),
            snapshot_path: None,
            cleanup_status: String::new(),
        }
    }

    fn script(name: &str, content: &str) -> ArchiveScript {
        ArchiveScript {
            filename: name.into(),
            content: content.into(),
        }
    }

    fn draft() -> ResearchArchive {
        ResearchArchive {
            id: "a1".into(),
            title: "Read counts".into(),
            report: "## Findings".into(),
            scripts: vec![script("count.sh", "wc -l"), script("plot.R", "plot(x)")],
            files: vec![
                file("counts.csv", "snapshot", true, 1024),
                file("reads.fq", "snapshot", false, 10),
                file("tmp.csv", "delete", true, 2048),
                // A delete the host could not have offered is not kept.
                file("input.tsv", "delete", false, 5),
            ],
            ..Default::default()
        }
    }

    fn review() -> Review {
        Review::new(draft(), "Read counts".into(), "## Findings".into())
    }

    #[test]
    fn the_files_and_their_choices_come_from_the_draft() {
        let review = review();
        assert_eq!(
            review.actions,
            [
                FileAction::Snapshot,
                FileAction::Snapshot,
                FileAction::Delete,
                FileAction::Snapshot,
            ]
        );
        assert_eq!(review.shown, Some(0));
        assert!(!review.edited("Read counts", "## Findings"));
        // A script is as the code editor holds it, so opening one in the
        // editor does not count as a change.
        let mut crlf = draft();
        crlf.scripts[0].content = "a\r\nb".into();
        let mut opened = Review::new(crlf, String::new(), String::new());
        assert_eq!(opened.scripts[0].content, "a\nb");
        opened.note_script("count.sh", "a\nb");
        assert!(!opened.edited("", ""));
        for action in FileAction::ALL {
            assert_eq!(FileAction::of(action.key()), action);
        }
        assert_eq!(FileAction::of("anything"), FileAction::Snapshot);
        // No script: none is shown.
        let none = Review::new(
            ResearchArchive {
                scripts: vec![],
                ..draft()
            },
            String::new(),
            String::new(),
        );
        assert_eq!(none.shown, None);
    }

    #[test]
    fn only_a_file_that_may_be_deleted_can_be_chosen_for_it() {
        let mut review = review();
        assert!(!review.choose(1, FileAction::Delete));
        assert_eq!(review.actions[1], FileAction::Snapshot);
        assert!(review.choose(1, FileAction::Reference));
        assert!(review.choose(0, FileAction::Delete));
        assert!(!review.choose(9, FileAction::Snapshot));
        assert!(review.edited("Read counts", "## Findings"));
    }

    #[test]
    fn the_freeze_request_carries_each_choice_and_the_changed_scripts() {
        let mut review = review();
        review.choose(1, FileAction::Reference);
        review.note_script("count.sh", "wc -l reads.fq");
        let request = review.request("  Read counts  ", "## Findings\nChecked.");
        assert_eq!(request.id, "a1");
        assert_eq!(request.title, "Read counts");
        assert_eq!(request.report, "## Findings\nChecked.");
        assert_eq!(request.scripts[0].content, "wc -l reads.fq");
        assert_eq!(request.scripts[1], script("plot.R", "plot(x)"));
        let choices: Vec<_> = request
            .files
            .iter()
            .map(|choice| (choice.path.as_str(), choice.action.as_str()))
            .collect();
        assert_eq!(
            choices,
            [
                ("counts.csv", "snapshot"),
                ("reads.fq", "reference"),
                ("tmp.csv", "delete"),
                ("input.tsv", "snapshot"),
            ]
        );
    }

    #[test]
    fn the_deletion_line_says_how_many_files_and_how_much() {
        let mut review = review();
        assert_eq!(
            review
                .deletions()
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            ["tmp.csv"]
        );
        assert_eq!(
            review.deletion_line(),
            "Selected for permanent deletion: 1 file (2.0 KB)"
        );
        review.choose(0, FileAction::Delete);
        assert_eq!(
            review.deletion_line(),
            "Selected for permanent deletion: 2 files (3.0 KB)"
        );
        review.choose(0, FileAction::Snapshot);
        review.choose(2, FileAction::Reference);
        assert!(review.deletions().is_empty());
        assert_eq!(
            review.deletion_line(),
            "No file is selected for permanent deletion."
        );
    }

    #[test]
    fn script_edits_stay_in_the_draft_across_switching() {
        let mut review = review();
        review.note_script("count.sh", "wc -l reads.fq");
        assert_eq!(review.show(1), Some(&script("plot.R", "plot(x)")));
        review.note_script("figure.R", "plot(y)");
        assert_eq!(review.show(0), Some(&script("count.sh", "wc -l reads.fq")));
        assert_eq!(review.scripts[1], script("figure.R", "plot(y)"));
        assert_eq!(review.show(5), None);
        assert_eq!(review.shown, Some(0));
        // The draft itself is as the host kept it.
        assert_eq!(review.draft.scripts[0].content, "wc -l");
        assert!(review.edited("Read counts", "## Findings"));
    }

    #[test]
    fn a_script_or_title_that_cannot_be_frozen_is_named_before_freezing() {
        let mut review = review();
        assert_eq!(review.problem("Read counts", "## Findings"), None);
        assert!(review.problem(" ", "## Findings").is_some());
        assert!(review.problem("Read counts", "\n").is_some());
        review.note_script("../count.sh", "wc -l");
        assert!(review.problem("Read counts", "## Findings").is_some());
        review.note_script("PLOT.r", "wc -l");
        // Two scripts may not share a name, in any case.
        assert!(review.problem("Read counts", "## Findings").is_some());
        review.note_script("count.sh", "wc -l");
        assert_eq!(review.problem("Read counts", "## Findings"), None);
    }

    #[test]
    fn the_confirmation_holds_only_for_the_review_as_it_was_read() {
        let mut review = review();
        assert!(!review.ready("Read counts", "## Findings"));
        review.accept("Read counts", "## Findings", true);
        assert!(review.ready("Read counts", "## Findings"));
        // Any change takes it back.
        assert!(!review.accepted("Read counts!", "## Findings"));
        review.choose(0, FileAction::Delete);
        assert!(!review.accepted("Read counts", "## Findings"));
        review.accept("Read counts", "## Findings", true);
        assert!(review.ready("Read counts", "## Findings"));
        review.freezing = true;
        assert!(!review.ready("Read counts", "## Findings"));
        review.freezing = false;
        review.accept("Read counts", "## Findings", false);
        assert!(!review.ready("Read counts", "## Findings"));
        // Nothing can be confirmed into freezing what the checks refuse.
        review.accept(" ", "## Findings", true);
        assert!(!review.ready(" ", "## Findings"));
    }

    #[test]
    fn notes_cleanup_and_highlighting_read_plainly() {
        let note = "Exclusive recorded creation / 本会话创建";
        assert_eq!(bilingual(note, false), "Exclusive recorded creation");
        assert_eq!(bilingual(note, true), "本会话创建");
        assert_eq!(bilingual("plain", true), "plain");
        assert_eq!(cleanup_line(""), None);
        assert_eq!(cleanup_line("deleted").as_deref(), Some("Deleted"));
        assert_eq!(
            cleanup_line("not deleted: changed since confirmation").as_deref(),
            Some("Not deleted: changed since confirmation")
        );
        let mut frozen = draft();
        assert!(cleanup_incomplete(&frozen));
        for file in &mut frozen.files {
            if file.action == "delete" {
                file.cleanup_status = "deleted".into();
            }
        }
        assert!(!cleanup_incomplete(&frozen));
        assert_eq!(highlighter("count.PY"), "python");
        assert_eq!(highlighter("plot.R"), "r");
        assert_eq!(highlighter("Makefile"), "text");
    }
}
