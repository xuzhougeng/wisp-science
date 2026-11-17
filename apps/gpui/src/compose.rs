//! What a message being written carries besides its text: project files and
//! references to conversations, skills, artifacts, environments, runtimes
//! and workflows. Kept per conversation, like the draft, and without
//! graphics.
use wisp_dto::{
    ComposerReferenceArg,
    native_conversations::{Item, ReadOnlyReason, Snapshot},
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
    /// A file the project recorded, by its artifact id.
    Artifact {
        id: String,
        name: String,
    },
    /// Where the turn's work is to run: this computer (`local`) or a saved
    /// host (`ssh:alias`).
    Environment {
        id: String,
        label: String,
    },
    /// An interpreter that keeps its variables between calls: `python` or
    /// `r`, on the environment `context_id`.
    Runtime {
        context_id: String,
        language: String,
        label: String,
    },
    /// A workflow template, by its id.
    Workflow {
        id: String,
        name: String,
    },
}

/// How a runtime's language is written.
pub fn language_name(language: &str) -> &str {
    match language {
        "python" => "Python",
        "r" => "R",
        other => other,
    }
}

/// What an environment is called when nothing better is known: this
/// computer, or a saved host by its alias.
pub fn environment_name(id: &str) -> String {
    match id {
        "local" => crate::i18n::tr("This computer").to_owned(),
        id => id.strip_prefix("ssh:").unwrap_or(id).to_owned(),
    }
}

impl Chip {
    pub fn label(&self) -> String {
        match self {
            Chip::File(path) => crate::files::file_name(path).to_owned(),
            Chip::Conversation { title, .. } => title.clone(),
            Chip::Skill(name) => name.clone(),
            Chip::Artifact { name, .. } | Chip::Workflow { name, .. } => name.clone(),
            Chip::Environment { label, .. } | Chip::Runtime { label, .. } => label.clone(),
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Chip::File(_) => "doc",
            Chip::Conversation { .. } => "chat",
            Chip::Skill(_) => "skill",
            Chip::Artifact { .. } => "layers",
            Chip::Environment { .. } => "server",
            Chip::Runtime { .. } => "terminal",
            Chip::Workflow { .. } => "branch",
        }
    }

    /// What the chip asks the host to tell the model; a file is not a
    /// reference but part of the message.
    pub fn reference(&self) -> Option<ComposerReferenceArg> {
        Some(match self {
            Chip::File(_) => return None,
            Chip::Conversation { id, .. } => ComposerReferenceArg::Session { id: id.clone() },
            Chip::Skill(name) => ComposerReferenceArg::Skill { name: name.clone() },
            Chip::Artifact { id, .. } => ComposerReferenceArg::Artifact { id: id.clone() },
            Chip::Environment { id, .. } => ComposerReferenceArg::Context { id: id.clone() },
            Chip::Runtime {
                context_id,
                language,
                ..
            } => ComposerReferenceArg::Runtime {
                context_id: context_id.clone(),
                language: language.clone(),
            },
            Chip::Workflow { id, .. } => ComposerReferenceArg::Workflow { id: id.clone() },
        })
    }

    /// The chip for a reference a message was sent with, called `name`
    /// when that is known. A project is not something this composer
    /// attaches.
    pub fn of_reference(reference: &ComposerReferenceArg, name: Option<String>) -> Option<Chip> {
        use crate::i18n::tr;
        Some(match reference {
            ComposerReferenceArg::Session { id } => Chip::Conversation {
                id: id.clone(),
                title: name.unwrap_or_else(|| tr("Conversation").to_owned()),
            },
            ComposerReferenceArg::Skill { name } => Chip::Skill(name.clone()),
            ComposerReferenceArg::Artifact { id } => Chip::Artifact {
                name: name.unwrap_or_else(|| id.clone()),
                id: id.clone(),
            },
            ComposerReferenceArg::Context { id } => Chip::Environment {
                label: name.unwrap_or_else(|| environment_name(id)),
                id: id.clone(),
            },
            ComposerReferenceArg::Runtime {
                context_id,
                language,
            } => Chip::Runtime {
                label: name.unwrap_or_else(|| runtime_chip_label(language, context_id)),
                context_id: context_id.clone(),
                language: language.clone(),
            },
            ComposerReferenceArg::Workflow { id } => Chip::Workflow {
                name: name.unwrap_or_else(|| id.clone()),
                id: id.clone(),
            },
            ComposerReferenceArg::Project { .. } => return None,
        })
    }
}

/// A runtime chip's words, as the WebView writes them: the language, then
/// where it runs.
pub fn runtime_chip_label(language: &str, context_id: &str) -> String {
    format!(
        "{} · {}",
        language_name(language),
        environment_name(context_id)
    )
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pending(Vec<Chip>);

impl Pending {
    /// Adds a chip unless the same thing is attached already: the same
    /// file, or what refers to the same thing whatever it is called now.
    pub fn add(&mut self, chip: Chip) -> bool {
        let same = |other: &Chip| match (other.reference(), chip.reference()) {
            (Some(a), Some(b)) => a == b,
            _ => *other == chip,
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

    /// Everything but files is context for this turn only: what a sent
    /// message shows it carried, besides its files.
    pub fn reference_chips(&self) -> Vec<Chip> {
        self.0
            .iter()
            .filter(|chip| !matches!(chip, Chip::File(_)))
            .cloned()
            .collect()
    }

    /// The chips of a message that was put aside: its files and what it
    /// referred to, each called what `name` finds for it.
    pub fn from_sent(
        attachments: &[String],
        references: &[ComposerReferenceArg],
        name: impl Fn(&ComposerReferenceArg) -> Option<String>,
    ) -> Self {
        let mut pending = Self::default();
        for path in attachments {
            pending.add(Chip::File(path.clone()));
        }
        for reference in references {
            if let Some(chip) = Chip::of_reference(reference, name(reference)) {
                pending.add(chip);
            }
        }
        pending
    }

    pub fn references(&self) -> Vec<ComposerReferenceArg> {
        self.0.iter().filter_map(Chip::reference).collect()
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

/// The name a region cut out of a picture is kept under in `uploads`, as
/// the WebView names it: `region_` and when it was cut, in UTC.
pub fn region_name(now_utc: chrono::NaiveDateTime) -> String {
    format!("region_{}.png", now_utc.format("%Y%m%d%H%M%S"))
}

/// Whether Cmd+V (Ctrl+V off macOS) was pressed with nothing else held.
pub fn is_paste(key: &str, primary: bool, shift: bool, others: bool) -> bool {
    key == "v" && primary && !shift && !others
}

/// Messages sent from this window with references, and what each carried,
/// oldest first, kept as long as the window: the host does not keep
/// references with a message, and neither does the WebView.
pub const SENT_KEPT: usize = 50;

/// The references the user message at `index` was sent with from this
/// window, if it was. Messages with the same words are told apart by
/// order, newest with newest.
pub fn sent_with<'a>(sent: &'a [(String, Vec<Chip>)], rows: &[Item], index: usize) -> &'a [Chip] {
    let Some(row) = rows.get(index).filter(|row| row.role == "user") else {
        return &[];
    };
    let text = user_text(row);
    let text = text.trim();
    let later = rows[index + 1..]
        .iter()
        .filter(|later| later.role == "user" && user_text(later).trim() == text)
        .count();
    let same: Vec<&(String, Vec<Chip>)> = sent.iter().filter(|(sent, _)| sent == text).collect();
    same.len()
        .checked_sub(later + 1)
        .and_then(|at| same.get(at))
        .map_or(&[], |(_, chips)| chips.as_slice())
}

/// Whether the composer's change from `before` to `after` is one `@` or `#`
/// typed where a word starts: what the WebView reads as asking to mention a
/// file, a conversation or a skill. The picker opens instead, and the
/// character is taken back out, so `before` is the text to keep.
pub fn mention_typed(before: &str, after: &str) -> bool {
    if after.chars().count() != before.chars().count() + 1 {
        return false;
    }
    let at = before
        .char_indices()
        .zip(after.char_indices())
        .find(|((_, old), (_, new))| old != new)
        .map_or(before.len(), |((index, _), _)| index);
    let Some(typed) = after[at..].chars().next() else {
        return false;
    };
    let rest = &after[at + typed.len_utf8()..];
    matches!(typed, '@' | '#')
        && rest == &before[at..]
        && before[..at].chars().last().is_none_or(char::is_whitespace)
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

/// The text of the composer once `text` is quoted, whole and line by line,
/// in front of what was being written: where the WebView puts a quote it
/// holds beside its composer (the notes on a picture's regions) when the
/// message is sent, so the model reads the same message.
pub fn with_quote_before(typed: &str, text: &str) -> String {
    let quoted = text
        .trim()
        .lines()
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let typed = typed.trim();
    if typed.is_empty() {
        format!("{quoted}\n\n")
    } else {
        format!("{quoted}\n\n{typed}")
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

/// Why a conversation cannot be written to, the cases the WebView desktop
/// tells apart. In place of its box the composer says which, and offers the
/// one way on that makes sense there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadOnly {
    /// A subagent's conversation: its agent takes instructions only from
    /// the conversation that started it, `parent` once the project's list
    /// of conversations has named it.
    Subagent { parent: Option<String> },
    /// Its research was frozen as an archive: it is a record now.
    Archived,
    /// The host marks it read-only and does not say why (an older host,
    /// or a reason this client does not know).
    Unexplained,
    /// The source of an exploration round that is not settled yet.
    FrozenMainline,
    /// An exploration that does not take messages (being made, being
    /// kept, failed or ended), with its status.
    ClosedExploration { status: String },
    /// A branch frozen as read-only history: brought back into the
    /// conversation it came from, or that conversation lost the turn it
    /// split from. `source` is the conversation it came from, when the
    /// project's list names one.
    FrozenBranch { source: Option<String> },
}

/// The way on that a read-only conversation offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadOnlyAction {
    /// Open this conversation: the one that started a subagent.
    Open(String),
    /// Open the research archive, which goes on in a new conversation.
    Archive,
}

/// Which read-only case the open conversation is in, if any. `parent` is
/// the conversation that started it, when the project's list says it is a
/// subagent's; `branch_source` the one it was branched from, when the list
/// says it is a branch. Why the host marks it read-only is the host's to
/// say: a reason it does not give is not guessed, so a subagent's
/// conversation opened before that list arrives never reads as archived.
pub fn read_only(
    snapshot: &Snapshot,
    parent: Option<&str>,
    branch_source: Option<&str>,
) -> Option<ReadOnly> {
    let parent = parent.map(str::to_owned);
    match (snapshot.read_only, snapshot.read_only_reason) {
        (true, Some(ReadOnlyReason::Archived)) => return Some(ReadOnly::Archived),
        (true, Some(ReadOnlyReason::FrozenBranch)) => {
            return Some(ReadOnly::FrozenBranch {
                source: branch_source.map(str::to_owned),
            });
        }
        (_, Some(ReadOnlyReason::Subagent)) => return Some(ReadOnly::Subagent { parent }),
        _ if parent.is_some() => return Some(ReadOnly::Subagent { parent }),
        (true, _) => return Some(ReadOnly::Unexplained),
        (false, _) => {}
    }
    let exploration = snapshot.exploration.as_ref()?;
    if exploration.source_frozen {
        return Some(ReadOnly::FrozenMainline);
    }
    // Only an active exploration accepts writes.
    exploration
        .summary
        .as_ref()
        .filter(|summary| summary.exploration.status != "active")
        .map(|summary| ReadOnly::ClosedExploration {
            status: summary.exploration.status.clone(),
        })
}

/// The one sentence that says why. `parent_title` names a subagent's
/// parent, when it is known.
pub fn read_only_reason(state: &ReadOnly, parent_title: &str) -> String {
    use crate::i18n::{fill, tr};
    match state {
        ReadOnly::Unexplained => tr("This conversation is read-only.").to_owned(),
        ReadOnly::Subagent { .. } if parent_title.trim().is_empty() => tr(
            "Subagent conversation: watch only. Send follow-ups from the conversation that started it.",
        )
        .to_owned(),
        ReadOnly::Subagent { .. } => fill(
            tr("A subagent of “{}”. Its agent takes instructions only from there; you can watch it here."),
            &[&parent_title],
        ),
        ReadOnly::Archived => tr(
            "This conversation's research is archived and it is read-only. Continue the research in a new conversation from the archive.",
        )
        .to_owned(),
        ReadOnly::FrozenMainline => tr(
            "This conversation is frozen until one exploration is kept or the round is abandoned; other conversations in the project can only read meanwhile.",
        )
        .to_owned(),
        ReadOnly::ClosedExploration { status } => fill(
            tr("This exploration is read-only ({})."),
            &[&crate::explore::status_label(status)],
        ),
        // The WebView's placeholder for a merged or orphaned branch.
        ReadOnly::FrozenBranch { .. } => tr("This branch is frozen as read-only history").to_owned(),
    }
}

/// The action the card offers, with its label. An exploration's round is
/// settled from the bar over the conversation, which stays, so the card
/// offers nothing of its own there.
pub fn read_only_action(state: &ReadOnly) -> Option<(&'static str, ReadOnlyAction)> {
    use crate::i18n::tr;
    match state {
        ReadOnly::Subagent { parent } => parent
            .clone()
            .map(|parent| (tr("Open the parent"), ReadOnlyAction::Open(parent))),
        ReadOnly::Archived => Some((
            tr("View the archive / Continue research"),
            ReadOnlyAction::Archive,
        )),
        // Work goes on in the conversation it came from, while there is one.
        ReadOnly::FrozenBranch { source } => source.clone().map(|source| {
            (
                tr("Open the source conversation"),
                ReadOnlyAction::Open(source),
            )
        }),
        ReadOnly::Unexplained | ReadOnly::FrozenMainline | ReadOnly::ClosedExploration { .. } => {
            None
        }
    }
}

/// What the open conversation's messages and menu offer that would change
/// it or start from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offered {
    /// Edit and send again under a message of yours.
    pub edit: bool,
    /// Branch from here under an answer.
    pub branch: bool,
    /// Review this turn, and Review in the menu.
    pub review: bool,
    /// Moving it to another project; a copy is always offered.
    pub move_away: bool,
}

/// What is offered, from why the conversation is read-only and whether it
/// is a branch, as the hosts allow it. As in the WebView: no message of a
/// read-only conversation is edited and sent again, and a branch, frozen or
/// not, is not branched again ("Conversation branches cannot be branched
/// again."). A frozen branch is not reviewed or moved away either, as the
/// Tauri desktop refuses it to its native clients. An exploration's
/// conversation that does not take messages is not branched, which the
/// host refuses too.
pub fn offered(read_only: Option<&ReadOnly>, branch: bool) -> Offered {
    let frozen = matches!(read_only, Some(ReadOnly::FrozenBranch { .. }));
    let exploring = matches!(
        read_only,
        Some(ReadOnly::FrozenMainline | ReadOnly::ClosedExploration { .. })
    );
    Offered {
        edit: read_only.is_none(),
        branch: !branch && !frozen && !exploring,
        review: !frozen,
        move_away: !frozen,
    }
}

/// The icon beside the sentence.
pub fn read_only_icon(state: &ReadOnly) -> &'static str {
    match state {
        ReadOnly::Subagent { .. } => "bot",
        ReadOnly::Archived => "archive",
        ReadOnly::Unexplained
        | ReadOnly::FrozenMainline
        | ReadOnly::ClosedExploration { .. }
        | ReadOnly::FrozenBranch { .. } => "lock",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn conversation(change: serde_json::Value) -> Snapshot {
        let mut value = json!({
            "schema": "test", "epoch": "host-1", "sequence": 1,
            "project_id": "p", "session_id": "s", "items": [],
            "next_before_seq": null, "running": false, "stopping": false,
            "read_only": false, "model_id": "offline", "request_id": null,
            "error": null, "approvals": []
        });
        for (key, changed) in change.as_object().unwrap() {
            value[key] = changed.clone();
        }
        serde_json::from_value(value).unwrap()
    }

    fn exploration(source_frozen: bool, status: Option<&str>) -> serde_json::Value {
        json!({"exploration": {
            "summary": status.map(|status| json!({
                "exploration": {
                    "id": "e1", "checkpoint_id": "k1", "frame_id": "s",
                    "name": "Try B", "status": status, "workspace_dir": "/w",
                    "workspace_backend": "copy", "scope_generation": 1,
                    "warnings_json": "[]", "created_at": 0, "updated_at": 0
                },
                "source_frame_id": "main", "checkpoint_user_index": 1,
                "isolation_summary_json": "{}"
            })),
            "active_count": 0, "source_frozen": source_frozen
        }})
    }

    #[test]
    fn a_read_only_conversation_says_why_and_offers_one_way_on() {
        // A conversation that can be written to is none of them.
        assert_eq!(read_only(&conversation(json!({})), None, None), None);
        assert_eq!(
            read_only(
                &conversation(exploration(false, Some("active"))),
                None,
                None
            ),
            None
        );

        // A subagent's, as the host says, with its parent once the list
        // of conversations names it.
        let told = json!({"read_only": true, "read_only_reason": "subagent"});
        let subagent = read_only(&conversation(told.clone()), Some("lead"), None).unwrap();
        assert_eq!(
            subagent,
            ReadOnly::Subagent {
                parent: Some("lead".into())
            }
        );
        // Known from the list alone, from a host that does not say why.
        assert_eq!(
            read_only(
                &conversation(json!({"read_only": true})),
                Some("lead"),
                None
            ),
            Some(subagent.clone())
        );
        assert_eq!(
            read_only_reason(&subagent, "Survey"),
            "A subagent of “Survey”. Its agent takes instructions only from there; you can watch it here."
        );
        assert!(read_only_reason(&subagent, " ").starts_with("Subagent conversation: watch only."));
        assert_eq!(
            read_only_action(&subagent),
            Some(("Open the parent", ReadOnlyAction::Open("lead".into())))
        );

        // Before the list has named its parent it is still a subagent's,
        // never an archive, and it offers nothing it cannot do.
        let unlisted = read_only(&conversation(told), None, None).unwrap();
        assert_eq!(unlisted, ReadOnly::Subagent { parent: None });
        assert!(read_only_reason(&unlisted, "").starts_with("Subagent conversation: watch only."));
        assert_eq!(read_only_action(&unlisted), None);

        // An archive only when the host says so.
        let archived = read_only(
            &conversation(json!({"read_only": true, "read_only_reason": "archived"})),
            None,
            None,
        )
        .unwrap();
        assert_eq!(archived, ReadOnly::Archived);
        assert!(read_only_reason(&archived, "").contains("research is archived"));
        assert_eq!(
            read_only_action(&archived),
            Some((
                "View the archive / Continue research",
                ReadOnlyAction::Archive
            ))
        );
        assert_eq!(read_only_icon(&archived), "archive");

        // Read-only for a reason not given, or not known here: it says so
        // and offers nothing.
        for unsaid in [
            json!({"read_only": true}),
            json!({"read_only": true, "read_only_reason": "a_later_reason"}),
        ] {
            let state = read_only(&conversation(unsaid), None, None).unwrap();
            assert_eq!(state, ReadOnly::Unexplained);
            assert_eq!(
                read_only_reason(&state, ""),
                "This conversation is read-only."
            );
            assert_eq!(read_only_action(&state), None);
        }

        // The source of an unsettled round, and an exploration that ended:
        // the bar over the conversation settles the round.
        let frozen = read_only(&conversation(exploration(true, None)), None, None).unwrap();
        assert_eq!(frozen, ReadOnly::FrozenMainline);
        assert!(read_only_reason(&frozen, "").starts_with("This conversation is frozen"));
        assert_eq!(read_only_action(&frozen), None);
        let ended = read_only(
            &conversation(exploration(false, Some("promoted"))),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            ended,
            ReadOnly::ClosedExploration {
                status: "promoted".into()
            }
        );
        assert_eq!(
            read_only_reason(&ended, ""),
            "This exploration is read-only (Ended)."
        );
        assert_eq!(read_only_action(&ended), None);
        assert_eq!(read_only_icon(&ended), "lock");
    }

    #[test]
    fn a_frozen_branch_says_so_and_opens_where_it_came_from() {
        let told = json!({"read_only": true, "read_only_reason": "frozen_branch"});
        // The list names where it came from, which it opens.
        let merged = read_only(&conversation(told.clone()), None, Some("main")).unwrap();
        assert_eq!(
            merged,
            ReadOnly::FrozenBranch {
                source: Some("main".into())
            }
        );
        assert_eq!(
            read_only_reason(&merged, ""),
            "This branch is frozen as read-only history"
        );
        assert_eq!(
            read_only_action(&merged),
            Some((
                "Open the source conversation",
                ReadOnlyAction::Open("main".into())
            ))
        );
        assert_eq!(read_only_icon(&merged), "lock");
        // A branch whose source was deleted is named by no list entry, so
        // it offers nothing.
        let orphaned = read_only(&conversation(told), None, None).unwrap();
        assert_eq!(orphaned, ReadOnly::FrozenBranch { source: None });
        assert_eq!(read_only_action(&orphaned), None);
        // An active branch is written to as any conversation is.
        assert_eq!(
            read_only(&conversation(json!({})), None, Some("main")),
            None
        );
        // A host that does not say why is not guessed at.
        assert_eq!(
            read_only(
                &conversation(json!({"read_only": true})),
                None,
                Some("main")
            ),
            Some(ReadOnly::Unexplained)
        );
    }

    /// The text of method `name` in `source`, from its signature to the
    /// brace that closes it at the `impl`'s indent.
    fn method<'a>(source: &'a str, name: &str) -> Option<&'a str> {
        let start = source.find(&format!("fn {name}("))?;
        let rest = &source[start..];
        Some(&rest[..rest.find("\n    }\n")?])
    }

    #[test]
    fn a_read_only_conversation_offers_no_edit_and_a_branch_no_second_branch() {
        let all = Offered {
            edit: true,
            branch: true,
            review: true,
            move_away: true,
        };
        assert_eq!(offered(None, false), all);
        // An active branch takes messages but is not branched again.
        assert_eq!(
            offered(None, true),
            Offered {
                branch: false,
                ..all
            }
        );
        // A frozen branch: nothing that changes it, a copy still.
        let frozen = ReadOnly::FrozenBranch { source: None };
        for branch in [true, false] {
            assert_eq!(
                offered(Some(&frozen), branch),
                Offered {
                    edit: false,
                    branch: false,
                    review: false,
                    move_away: false,
                }
            );
        }
        // Other read-only conversations: no edit; reviewed and moved as
        // before, and branched unless an exploration holds them.
        for state in [
            ReadOnly::Archived,
            ReadOnly::Subagent { parent: None },
            ReadOnly::Unexplained,
        ] {
            assert_eq!(
                offered(Some(&state), false),
                Offered { edit: false, ..all },
                "{state:?}"
            );
        }
        for state in [
            ReadOnly::FrozenMainline,
            ReadOnly::ClosedExploration {
                status: "promoting".into(),
            },
        ] {
            assert_eq!(
                offered(Some(&state), false),
                Offered {
                    edit: false,
                    branch: false,
                    ..all
                },
                "{state:?}"
            );
        }
    }

    #[test]
    fn every_way_into_a_read_only_conversation_is_shut() {
        // The send key, Send, Queue and a typed command all go through
        // `send`; answers picked on cards through `deliver`; files through
        // the picker, a drop or a paste; references through the add-context
        // box; and the `/` menu through `run_slash`.
        let sources = [
            include_str!("workspace.rs"),
            include_str!("workspace/queue_view.rs"),
            include_str!("workspace/palette_view.rs"),
            include_str!("workspace/slash_view.rs"),
        ];
        for name in [
            "send",
            "deliver",
            "enqueue",
            "attach_files",
            "attach_dropped",
            "paste_picture",
            "toggle_attach_palette",
            "run_slash",
        ] {
            let body = sources
                .iter()
                .find_map(|source| method(source, name))
                .unwrap_or_else(|| panic!("no fn {name}"));
            assert!(body.contains("self.read_only()"), "{name} is not shut");
        }
        // And the box itself is not drawn: the card stands in its place.
        let composer = method(include_str!("workspace/shell.rs"), "render_composer").unwrap();
        let card = composer.find("self.read_only_card(").expect("the card");
        let editor = composer
            .find("surfaces::input(&self.composer, cx)")
            .unwrap();
        assert!(card < editor);
    }

    #[test]
    fn an_at_or_hash_typed_where_a_word_starts_asks_for_a_mention() {
        assert!(mention_typed("", "@"));
        assert!(mention_typed("compare ", "compare #"));
        assert!(mention_typed("see  and", "see @ and"));
        assert!(mention_typed("行 ", "行 @"));
        // Inside a word, an address or a tag, it is just a character.
        assert!(!mention_typed("me", "me@"));
        assert!(!mention_typed("issue", "issue#"));
        assert!(!mention_typed("compare ", "compare x"));
        // Pasted or deleted text is not typing one character.
        assert!(!mention_typed("", "@@"));
        assert!(!mention_typed("a @", "a "));
        assert!(!mention_typed("", ""));
    }

    #[test]
    fn a_sent_message_shows_the_references_it_carried() {
        let row = |role: &str, text: &str| -> Item {
            serde_json::from_value(json!({
                "role": role, "text": text, "tool_name": null, "input": null,
                "ok": null, "status": null
            }))
            .unwrap()
        };
        let skill = Chip::Skill("stats".into());
        let earlier = Chip::Conversation {
            id: "c1".into(),
            title: "Counts".into(),
        };
        let sent = vec![
            ("compare".to_string(), vec![skill.clone()]),
            ("compare".to_string(), vec![earlier.clone()]),
        ];
        let rows = [
            row("user", "compare"),
            row("assistant", "Done."),
            row("user", "compare"),
            row("user", "thanks"),
        ];
        // The same words twice: newest with newest.
        assert_eq!(sent_with(&sent, &rows, 2), [earlier]);
        assert_eq!(sent_with(&sent, &rows, 0), [skill]);
        // Sent with nothing, not a message, or not there.
        assert!(sent_with(&sent, &rows, 3).is_empty());
        assert!(sent_with(&sent, &rows, 1).is_empty());
        assert!(sent_with(&sent, &rows, 9).is_empty());
        // An older message with the same words than were sent from here.
        let more = [
            row("user", "compare"),
            row("user", "compare"),
            row("user", "compare"),
        ];
        assert!(sent_with(&sent, &more, 0).is_empty());
        let pending = Pending(vec![
            Chip::File("a.csv".into()),
            Chip::Skill("stats".into()),
        ]);
        assert_eq!(pending.reference_chips(), [Chip::Skill("stats".into())]);
    }

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
        assert_eq!(region_name(at), "region_20261115090507.png");
    }

    #[test]
    fn notes_on_a_picture_go_whole_in_front_of_what_is_written() {
        let notes =
            "Revision notes on image `a.png`:\n1. (x=0.100, y=0.200, w=0.300, h=0.400) darker";
        assert_eq!(
            with_quote_before("", notes),
            "> Revision notes on image `a.png`:\n> 1. (x=0.100, y=0.200, w=0.300, h=0.400) darker\n\n"
        );
        assert_eq!(
            with_quote_before("Fix these.  \n", notes),
            "> Revision notes on image `a.png`:\n> 1. (x=0.100, y=0.200, w=0.300, h=0.400) darker\n\nFix these."
        );
        // However many notes there are, none is cut.
        let long = "n\n".repeat(2_000);
        assert_eq!(with_quote_before("", &long).matches("> n").count(), 2_000);
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
    fn a_message_put_aside_gets_its_chips_back() {
        let sent = [
            ComposerReferenceArg::Session { id: "s1".into() },
            ComposerReferenceArg::Skill {
                name: "deseq2".into(),
            },
            ComposerReferenceArg::Project { id: "p".into() },
            ComposerReferenceArg::Artifact { id: "a1".into() },
            ComposerReferenceArg::Context {
                id: "ssh:gpu".into(),
            },
            ComposerReferenceArg::Runtime {
                context_id: "local".into(),
                language: "r".into(),
            },
            ComposerReferenceArg::Workflow { id: "w1".into() },
        ];
        let pending = Pending::from_sent(
            &["data/qc.csv".into()],
            &sent,
            |reference| match reference {
                ComposerReferenceArg::Session { id } => Some(format!("Title of {id}")),
                ComposerReferenceArg::Artifact { .. } => Some("plot.png".into()),
                _ => None,
            },
        );
        assert_eq!(
            pending.chips(),
            [
                Chip::File("data/qc.csv".into()),
                Chip::Conversation {
                    id: "s1".into(),
                    title: "Title of s1".into(),
                },
                Chip::Skill("deseq2".into()),
                Chip::Artifact {
                    id: "a1".into(),
                    name: "plot.png".into(),
                },
                // Named from what the reference says when nothing better
                // is known.
                Chip::Environment {
                    id: "ssh:gpu".into(),
                    label: "gpu".into(),
                },
                Chip::Runtime {
                    context_id: "local".into(),
                    language: "r".into(),
                    label: "R · This computer".into(),
                },
                Chip::Workflow {
                    id: "w1".into(),
                    name: "w1".into(),
                },
            ]
        );
        // What it sends is what it sent, but for the project, which this
        // composer does not attach.
        assert_eq!(pending.attachments(), ["data/qc.csv"]);
        let mut expected = sent.to_vec();
        expected.remove(2);
        assert_eq!(pending.references(), expected);
    }

    #[test]
    fn each_kind_of_chip_sends_its_reference_and_shows_its_own_icon() {
        let chips = [
            Chip::Artifact {
                id: "a1".into(),
                name: "fig1.png".into(),
            },
            Chip::Environment {
                id: "local".into(),
                label: "This computer".into(),
            },
            Chip::Runtime {
                context_id: "local".into(),
                language: "python".into(),
                label: runtime_chip_label("python", "local"),
            },
            Chip::Workflow {
                id: "w1".into(),
                name: "Screen".into(),
            },
        ];
        let mut pending = Pending::default();
        for chip in chips.clone() {
            assert!(pending.add(chip));
        }
        // The same thing under another name is still the same thing.
        assert!(!pending.add(Chip::Artifact {
            id: "a1".into(),
            name: "renamed.png".into(),
        }));
        assert!(!pending.add(Chip::Workflow {
            id: "w1".into(),
            name: "Renamed".into(),
        }));
        // Another language on the same computer is another runtime.
        assert!(pending.add(Chip::Runtime {
            context_id: "local".into(),
            language: "r".into(),
            label: runtime_chip_label("r", "local"),
        }));
        assert!(pending.attachments().is_empty());
        assert_eq!(
            pending.references()[..4],
            [
                ComposerReferenceArg::Artifact { id: "a1".into() },
                ComposerReferenceArg::Context { id: "local".into() },
                ComposerReferenceArg::Runtime {
                    context_id: "local".into(),
                    language: "python".into(),
                },
                ComposerReferenceArg::Workflow { id: "w1".into() },
            ]
        );
        let labels: Vec<_> = pending.chips().iter().map(Chip::label).collect();
        assert_eq!(
            labels,
            [
                "fig1.png",
                "This computer",
                "Python · This computer",
                "Screen",
                "R · This computer"
            ]
        );
        let mut icons: Vec<_> = chips.iter().map(Chip::icon).collect();
        icons.extend([
            Chip::File("a".into()).icon(),
            Chip::Skill("s".into()).icon(),
            Chip::Conversation {
                id: "c".into(),
                title: "c".into(),
            }
            .icon(),
        ]);
        let count = icons.len();
        icons.sort_unstable();
        icons.dedup();
        assert_eq!(icons.len(), count);
        // They are what a sent message shows it carried.
        assert_eq!(pending.reference_chips().len(), 5);
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
