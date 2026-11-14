//! Slash commands: a message that is only a command, typed into the
//! composer, does something in the app instead of being sent.
//!
//! Only a known command at the very start counts, so a message that
//! begins with a path (`/data/reads.fq looks odd`) is still a message.

/// What a slash command does here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slash {
    Review,
    Trajectory,
    /// What the model is sent.
    Context,
    /// A question asked aside; what follows the command is the question.
    Btw,
    Plan,
    SaveAsSkill,
    Skills,
    Files,
    Upload,
    Memo,
    Remember,
    Archive,
    /// Shortens what the model is sent, now.
    Compact,
    /// Lists the commands.
    Help,
}

/// The commands with what each does, in the order they are listed.
pub const COMMANDS: [(&str, Slash, &str); 14] = [
    ("/review", Slash::Review, "Review this conversation"),
    (
        "/trajectory",
        Slash::Trajectory,
        "Trajectory of this conversation",
    ),
    ("/context", Slash::Context, "What the model is sent"),
    ("/btw", Slash::Btw, "Ask aside about this conversation"),
    ("/plan", Slash::Plan, "Switch plan first on or off"),
    (
        "/save-as-skill",
        Slash::SaveAsSkill,
        "Save this conversation as a skill",
    ),
    ("/skills", Slash::Skills, "Show the skills in Settings"),
    ("/files", Slash::Files, "Show or hide Files"),
    ("/upload", Slash::Upload, "Attach files to the message"),
    ("/memo", Slash::Memo, "Show memos"),
    ("/remember", Slash::Remember, "Show memory"),
    (
        "/archive",
        Slash::Archive,
        "Archive this conversation's research",
    ),
    ("/compact", Slash::Compact, "Compact this conversation now"),
    ("/help", Slash::Help, "List these commands"),
];

/// Commands the WebView desktop has that this one does not yet.
const NOT_HERE: [&str; 5] = ["/timer", "/fork", "/rewind", "/permission", "/share"];

/// What a typed message is, when it starts with a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Typed {
    /// A command of this desktop, with what was typed after it.
    Run(Slash, String),
    /// A command the WebView desktop has and this one does not.
    NotHere(String),
}

/// Reads a command from the start of `text`. `None` is an ordinary
/// message, a path included.
pub fn parse(text: &str) -> Option<Typed> {
    let text = text.trim();
    let (word, rest) = match text.split_once(char::is_whitespace) {
        Some((word, rest)) => (word, rest.trim()),
        None => (text, ""),
    };
    let word = word.to_ascii_lowercase();
    if let Some((_, slash, _)) = COMMANDS.iter().find(|(name, ..)| *name == word) {
        return Some(Typed::Run(*slash, rest.to_owned()));
    }
    NOT_HERE
        .contains(&word.as_str())
        .then(|| Typed::NotHere(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_known_command_at_the_start_is_a_command() {
        assert_eq!(
            parse(" /review "),
            Some(Typed::Run(Slash::Review, String::new()))
        );
        assert_eq!(
            parse("/REVIEW"),
            Some(Typed::Run(Slash::Review, String::new()))
        );
        assert_eq!(
            parse("/btw  why did the fit fail?\nand when"),
            Some(Typed::Run(
                Slash::Btw,
                "why did the fit fail?\nand when".into()
            ))
        );
        assert_eq!(
            parse("/timer 1h check"),
            Some(Typed::NotHere("/timer".into()))
        );
        // Paths, unknown words and a slash later in the text are messages.
        for message in [
            "/data/reads.fq looks odd",
            "/reviewer",
            "/unknown",
            "please /review this",
            "/",
            "",
        ] {
            assert_eq!(parse(message), None, "{message}");
        }
    }

    #[test]
    fn every_command_is_listed_once_and_starts_with_a_slash() {
        for (index, (name, slash, about)) in COMMANDS.iter().enumerate() {
            assert!(name.starts_with('/') && !about.is_empty());
            assert_eq!(parse(name), Some(Typed::Run(*slash, String::new())));
            assert!(
                COMMANDS[index + 1..]
                    .iter()
                    .all(|(other, ..)| other != name)
            );
            assert!(!NOT_HERE.contains(name));
        }
    }
}

/// What a finished compaction says.
pub fn compacted_note(before: usize, after: usize, summarised: bool) -> String {
    use crate::i18n::{fill, tr};
    let how = if summarised {
        tr("Earlier turns were folded into a summary.")
    } else {
        tr("Old tool output was pruned; no summary was needed.")
    };
    format!(
        "{} {how}",
        fill(
            tr(
                "Compacted: about {} tokens before, {} after. The whole history is archived in the project."
            ),
            &[&before, &after],
        )
    )
}

#[cfg(test)]
mod note_tests {
    #[test]
    fn a_compaction_says_what_it_did() {
        assert_eq!(
            super::compacted_note(90_000, 12_000, true),
            "Compacted: about 90000 tokens before, 12000 after. The whole history is archived in the project. Earlier turns were folded into a summary."
        );
        assert!(super::compacted_note(10, 9, false).ends_with("no summary was needed."));
    }
}
