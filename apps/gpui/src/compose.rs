//! What a message being written carries besides its text: project files and
//! references to conversations and skills. Kept per conversation, like the
//! draft, and without graphics.
use wisp_dto::{ComposerReferenceArg, native_conversations::Item};

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
}
