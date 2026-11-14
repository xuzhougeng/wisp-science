//! What a message being written carries besides its text: project files and
//! references to conversations and skills. Kept per conversation, like the
//! draft, and without graphics.
use wisp_dto::{
    ComposerReferenceArg,
    native_conversations::{Item, Snapshot},
};

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}
