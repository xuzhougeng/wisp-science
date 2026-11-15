//! What a message being written carries besides its text: project files and
//! references to conversations and skills. Kept per conversation, like the
//! draft, and without graphics.
use wisp_dto::{
    ComposerReferenceArg,
    native_conversations::{Item, Snapshot},
};

/// What asks the model to turn the conversation into a skill. The wording
/// is the WebView desktop's; the bundled `skill-creator` skill does the work.
pub const SKILL_PROMPT: &str = "Use the skill-creator skill to create a new, reusable skill \
    from scratch that distills the workflow and know-how from this session.";

/// The composer's text once the skill request is put in it: the request
/// alone, or after whatever was already being written, which is kept.
pub fn with_skill_prompt(typed: &str) -> String {
    let typed = typed.trim();
    if typed.is_empty() || typed == SKILL_PROMPT || typed.ends_with(SKILL_PROMPT) {
        // Asking twice does not write it twice.
        return if typed.is_empty() {
            SKILL_PROMPT.to_owned()
        } else {
            typed.to_owned()
        };
    }
    format!("{typed}\n\n{SKILL_PROMPT}")
}

/// The composer's text once a quick action is picked: its request, after
/// whatever was already being written. The request's end is kept as it
/// is, because that is where the user goes on typing.
pub fn with_quick_action(typed: &str, prompt: &str) -> String {
    let typed = typed.trim();
    if typed.is_empty() || typed.ends_with(prompt.trim_end()) {
        return if typed.is_empty() {
            prompt.to_owned()
        } else {
            typed.to_owned()
        };
    }
    format!("{typed}\n\n{prompt}")
}

/// The questions to offer above the composer: the ones suggested after the
/// latest answer. They are kept back while a turn runs, while a tool waits
/// for approval and in a conversation that cannot be written to, because a
/// question could not be asked then.
pub fn suggestions(snapshot: &Snapshot) -> &[String] {
    if snapshot.running || snapshot.read_only || !snapshot.approvals.is_empty() {
        return &[];
    }
    &snapshot.follow_ups
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chip {
    /// Project-relative path.
    File(String),
    Conversation {
        id: String,
        title: String,
    },
    Skill(String),
}

impl Chip {
    pub fn label(&self) -> String {
        match self {
            Chip::File(path) => crate::files::file_name(path).to_owned(),
            Chip::Conversation { title, .. } => title.clone(),
            Chip::Skill(name) => name.clone(),
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Chip::File(_) => "doc",
            Chip::Conversation { .. } => "chat",
            Chip::Skill(_) => "skill",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pending(Vec<Chip>);

impl Pending {
    /// Adds a chip unless the same thing is attached already.
    pub fn add(&mut self, chip: Chip) -> bool {
        let same = |other: &Chip| match (other, &chip) {
            (Chip::Conversation { id: a, .. }, Chip::Conversation { id: b, .. }) => a == b,
            (a, b) => a == b,
        };
        if self.0.iter().any(same) {
            return false;
        }
        self.0.push(chip);
        true
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.0.len() {
            self.0.remove(index);
        }
    }

    pub fn chips(&self) -> &[Chip] {
        &self.0
    }

    /// Files travel with the saved message.
    pub fn attachments(&self) -> Vec<String> {
        self.0
            .iter()
            .filter_map(|chip| match chip {
                Chip::File(path) => Some(path.clone()),
                _ => None,
            })
            .collect()
    }

    /// Conversations and skills are context for this turn only.
    pub fn references(&self) -> Vec<ComposerReferenceArg> {
        self.0
            .iter()
            .filter_map(|chip| match chip {
                Chip::Conversation { id, .. } => {
                    Some(ComposerReferenceArg::Session { id: id.clone() })
                }
                Chip::Skill(name) => Some(ComposerReferenceArg::Skill { name: name.clone() }),
                Chip::File(_) => None,
            })
            .collect()
    }
}

/// A user message as it should read in the transcript: the files it carried
/// are shown as chips, so the block that lists them is left out of the text.
/// How many files one drop may attach.
pub const DROP_LIMIT: usize = 20;

/// What a drop onto the conversation attaches: the regular files among
/// `paths`, in order and without repeats, up to [`DROP_LIMIT`], and how
/// many of the dropped things were left out (folders, and files past the
/// limit). `is_file` says whether a path is a regular file.
pub fn dropped_files(
    paths: &[std::path::PathBuf],
    is_file: impl Fn(&std::path::Path) -> bool,
) -> (Vec<std::path::PathBuf>, usize) {
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut left_out = 0;
    for path in paths {
        if files.contains(path) {
            continue;
        }
        if is_file(path) && files.len() < DROP_LIMIT {
            files.push(path.clone());
        } else {
            left_out += 1;
        }
    }
    (files, left_out)
}

/// What is said when part of a drop was not attached.
pub fn drop_note(left_out: usize) -> Option<String> {
    match left_out {
        0 => None,
        1 => Some(format!(
            "1 item was not attached: folders are not attached, and a drop takes at most \
             {DROP_LIMIT} files."
        )),
        n => Some(format!(
            "{n} items were not attached: folders are not attached, and a drop takes at most \
             {DROP_LIMIT} files."
        )),
    }
}

/// The name a pasted picture is kept under in `uploads`: when it was
/// pasted, with the extension of its format.
pub fn pasted_name(extension: &str, now: chrono::NaiveDateTime) -> String {
    format!("pasted-{}.{extension}", now.format("%Y%m%d-%H%M%S"))
}

/// Whether Cmd+V (Ctrl+V off macOS) was pressed with nothing else held.
pub fn is_paste(key: &str, primary: bool, shift: bool, others: bool) -> bool {
    key == "v" && primary && !shift && !others
}

pub fn user_text(item: &Item) -> String {
    if item.attachments.is_empty() {
        return item.text.clone();
    }
    item.text
        .split("\n\n")
        .filter(|block| !block.starts_with("Uploaded files: "))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// What undoing a turn did to the project's files, in a line or two;
/// nothing when the turn changed no file.
pub fn undo_note(files: &wisp_server::desktop::TurnFiles) -> Option<String> {
    use crate::i18n::{fill, tr};
    let mut parts = vec![];
    if !files.restored.is_empty() {
        parts.push(fill(tr("Put back: {}."), &[&files.restored.join(", ")]));
    }
    if !files.removed.is_empty() {
        parts.push(fill(
            tr("Moved to the project's trash: {}."),
            &[&files.removed.join(", ")],
        ));
    }
    if !files.left.is_empty() {
        let left: Vec<String> = files
            .left
            .iter()
            .map(|(path, why)| format!("{path} ({why})"))
            .collect();
        parts.push(fill(tr("Left as they are: {}."), &[&left.join(", ")]));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Where the end of `text` is, as a line and a character on it (both
/// from 0): where the caret goes when the composer is filled for you to
/// go on typing.
pub fn end_of(text: &str) -> (u32, u32) {
    let line = text.matches('\n').count();
    let last = text.rsplit('\n').next().unwrap_or_default();
    (line as u32, last.chars().count() as u32)
}

/// How much of a message a quote keeps.
const QUOTE_CHARS: usize = 1_500;

/// A message's text as a Markdown quote, cut to leave room for what is
/// said about it.
pub fn quote(text: &str) -> String {
    let text = text.trim();
    let mut kept: String = text.chars().take(QUOTE_CHARS).collect();
    if text.chars().count() > QUOTE_CHARS {
        kept.truncate(kept.trim_end().len());
        kept.push('…');
    }
    kept.lines()
        .map(|line| {
            if line.trim().is_empty() {
                ">".to_owned()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A message quoted on one line, for a single-line field such as the
/// aside's question (which cannot hold a line break), after what is
/// already written there.
pub fn with_inline_quote(typed: &str, message: &str) -> String {
    const SHORT: usize = 280;
    let line = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut kept: String = line.chars().take(SHORT).collect();
    if line.chars().count() > SHORT {
        kept.truncate(kept.trim_end().len());
        kept.push('…');
    }
    let typed = typed.trim_end();
    if typed.is_empty() {
        format!("“{kept}” ")
    } else {
        format!("{typed} “{kept}” ")
    }
}

/// The text of a field once a quote of `message` is added after what was
/// being written there, ready for the words about it.
pub fn with_quote(typed: &str, message: &str) -> String {
    let typed = typed.trim_end();
    if typed.is_empty() {
        format!("{}\n\n", quote(message))
    } else {
        format!("{typed}\n\n{}\n\n", quote(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_pasted_picture_is_named_for_when_it_was_pasted() {
        let at = chrono::NaiveDate::from_ymd_opt(2026, 11, 15)
            .unwrap()
            .and_hms_opt(9, 5, 7)
            .unwrap();
        assert_eq!(pasted_name("png", at), "pasted-20261115-090507.png");
        assert!(is_paste("v", true, false, false));
        assert!(!is_paste("v", false, false, false));
        assert!(!is_paste("v", true, true, false));
        assert!(!is_paste("c", true, false, false));
    }

    #[test]
    fn the_skill_request_is_added_once_and_keeps_what_was_typed() {
        assert_eq!(with_skill_prompt("  "), SKILL_PROMPT);
        let after_draft = with_skill_prompt(" Focus on the QC steps. ");
        assert_eq!(
            after_draft,
            format!("Focus on the QC steps.\n\n{SKILL_PROMPT}")
        );
        // Pressed again, nothing is doubled.
        assert_eq!(with_skill_prompt(&after_draft), after_draft);
        assert_eq!(with_skill_prompt(SKILL_PROMPT), SKILL_PROMPT);
        assert!(SKILL_PROMPT.contains("skill-creator"));
    }

    #[test]
    fn undoing_a_turn_says_what_happened_to_each_file() {
        use wisp_server::desktop::TurnFiles;
        assert_eq!(undo_note(&TurnFiles::default()), None);
        let files = TurnFiles {
            restored: vec!["fit.py".into()],
            removed: vec!["out.csv".into(), "plot.png".into()],
            left: vec![(
                "notes.md".into(),
                "was changed again after this turn".into(),
            )],
        };
        assert_eq!(
            undo_note(&files).unwrap(),
            "Put back: fit.py. Moved to the project's trash: out.csv, plot.png. \
             Left as they are: notes.md (was changed again after this turn)."
        );
    }

    #[test]
    fn the_end_of_a_text_is_its_last_line_and_its_length() {
        assert_eq!(end_of(""), (0, 0));
        assert_eq!(end_of("Result: "), (0, 8));
        assert_eq!(end_of("Give three.\n\nResult: "), (2, 8));
        assert_eq!(end_of("中文\n"), (1, 0));
        assert_eq!(end_of("一\n二三"), (1, 2));
    }

    #[test]
    fn a_quick_action_goes_after_what_was_typed_and_keeps_its_open_end() {
        assert_eq!(
            with_quick_action("  ", "Check the QC of: "),
            "Check the QC of: "
        );
        assert_eq!(
            with_quick_action("sample S04 ", "Check the QC of: "),
            "sample S04\n\nCheck the QC of: "
        );
        // Picked twice, it is there once.
        assert_eq!(
            with_quick_action("Check the QC of: ", "Check the QC of: "),
            "Check the QC of:"
        );
    }

    #[test]
    fn suggestions_are_offered_only_when_a_question_could_be_asked() {
        let snapshot = |change: serde_json::Value| -> Snapshot {
            let mut value = json!({
                "schema": "test", "epoch": "host-1", "sequence": 1,
                "project_id": "p", "session_id": "s", "items": [],
                "next_before_seq": null, "running": false, "stopping": false,
                "read_only": false, "model_id": "offline", "request_id": null,
                "error": null, "approvals": [],
                "follow_ups": ["Why?", "How many?"]
            });
            for (key, changed) in change.as_object().unwrap() {
                value[key] = changed.clone();
            }
            serde_json::from_value(value).unwrap()
        };
        assert_eq!(suggestions(&snapshot(json!({}))), ["Why?", "How many?"]);
        assert!(suggestions(&snapshot(json!({"follow_ups": []}))).is_empty());
        // A host that suggests nothing sends no list at all.
        let mut bare = serde_json::to_value(snapshot(json!({"follow_ups": []}))).unwrap();
        assert!(bare.get("follow_ups").is_none());
        bare["sequence"] = json!(2);
        assert!(suggestions(&serde_json::from_value::<Snapshot>(bare).unwrap()).is_empty());
        // Nothing could be asked now: they wait.
        assert!(suggestions(&snapshot(json!({"running": true}))).is_empty());
        assert!(suggestions(&snapshot(json!({"read_only": true}))).is_empty());
        let waiting = json!({"approvals": [{
            "approval_id": "a1", "frame_id": "s", "message": "Run tool 'write'?",
            "tool": "write", "preview": ""
        }]});
        assert!(suggestions(&snapshot(waiting)).is_empty());
    }

    #[test]
    fn chips_are_unique_and_split_into_files_and_references() {
        let mut pending = Pending::default();
        assert!(pending.chips().is_empty());
        assert!(pending.add(Chip::File("data/counts.csv".into())));
        assert!(!pending.add(Chip::File("data/counts.csv".into())));
        assert!(pending.add(Chip::Skill("align".into())));
        assert!(pending.add(Chip::Conversation {
            id: "s1".into(),
            title: "Earlier".into()
        }));
        // The same conversation under a new title is still the same one.
        assert!(!pending.add(Chip::Conversation {
            id: "s1".into(),
            title: "Renamed".into()
        }));
        assert_eq!(pending.attachments(), ["data/counts.csv"]);
        assert_eq!(
            pending.references(),
            [
                ComposerReferenceArg::Skill {
                    name: "align".into()
                },
                ComposerReferenceArg::Session { id: "s1".into() },
            ]
        );
        let labels: Vec<_> = pending.chips().iter().map(Chip::label).collect();
        assert_eq!(labels, ["counts.csv", "align", "Earlier"]);
        pending.remove(1);
        pending.remove(9);
        assert_eq!(pending.chips().len(), 2);
        assert!(pending.references().len() == 1);
    }

    #[test]
    fn the_file_list_is_shown_as_chips_not_as_text() {
        let item = |text: &str, attachments: serde_json::Value| -> Item {
            serde_json::from_value(json!({
                "role": "user", "text": text, "tool_name": null, "input": null,
                "ok": null, "status": null, "attachments": attachments
            }))
            .unwrap()
        };
        let with_files = item(
            "Compare these.\n\nSecond paragraph.\n\nUploaded files: a.csv, b.csv",
            json!(["a.csv", "b.csv"]),
        );
        assert_eq!(
            user_text(&with_files),
            "Compare these.\n\nSecond paragraph."
        );
        assert_eq!(
            user_text(&item("Uploaded files: a.csv", json!(["a.csv"]))),
            ""
        );
        // Without attachments the text is left exactly as written.
        let plain = item("Uploaded files: is what I typed", json!([]));
        assert_eq!(user_text(&plain), "Uploaded files: is what I typed");
    }

    #[test]
    fn a_drop_attaches_its_files_and_says_what_it_left_out() {
        use std::path::{Path, PathBuf};
        let path = |name: &str| PathBuf::from(format!("/data/{name}"));
        let is_file = |path: &Path| path.extension().is_some();
        let dropped = [path("a.csv"), path("folder"), path("b.csv"), path("a.csv")];
        let (files, left_out) = dropped_files(&dropped, is_file);
        // Files keep their order, a repeat is one file, a folder is left out.
        assert_eq!(files, [path("a.csv"), path("b.csv")]);
        assert_eq!(left_out, 1);
        assert!(drop_note(1).unwrap().starts_with("1 item was not attached"));
        assert_eq!(drop_note(0), None);

        let many: Vec<PathBuf> = (0..DROP_LIMIT + 3)
            .map(|n| path(&format!("{n}.csv")))
            .collect();
        let (files, left_out) = dropped_files(&many, is_file);
        assert_eq!((files.len(), left_out), (DROP_LIMIT, 3));
        assert!(
            drop_note(3)
                .unwrap()
                .starts_with("3 items were not attached")
        );
        assert_eq!(dropped_files(&[], is_file), (vec![], 0));
    }

    #[test]
    fn a_message_is_quoted_line_by_line_after_what_is_written() {
        assert_eq!(quote("First line\n\nSecond"), "> First line\n>\n> Second");
        assert_eq!(with_quote("", "S1 is low."), "> S1 is low.\n\n");
        assert_eq!(
            with_quote("Why?  \n", "S1 is low."),
            "Why?\n\n> S1 is low.\n\n"
        );
        // A long answer is cut, and says so.
        let long = "word ".repeat(1_000);
        let cut = quote(&long);
        assert!(cut.ends_with('…') && cut.chars().count() < 1_600);
        // On one line for a field that cannot hold a line break.
        let inline = with_inline_quote("", "S1 is low.\n\nCheck it.");
        assert_eq!(inline, "“S1 is low. Check it.” ");
        assert!(!with_inline_quote("Why", &long).contains('\n'));
        assert!(with_inline_quote("Why", &long).starts_with("Why “"));
    }
}
