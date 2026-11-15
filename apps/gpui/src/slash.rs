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
    /// Repeats a prompt in this conversation on an interval.
    Timer,
    /// Branches the conversation as it stands.
    Fork,
    /// Goes back to the latest message.
    Rewind,
    /// Switches full access on or off.
    Permission,
    /// Saves what was said as one HTML page to pass on.
    Share,
    /// Lists the commands.
    Help,
}

/// The commands with what each does, in the order they are listed.
pub const COMMANDS: [(&str, Slash, &str); 19] = [
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
    (
        "/timer",
        Slash::Timer,
        "Repeat a prompt here: /timer 30m <prompt>, /timer off",
    ),
    ("/fork", Slash::Fork, "Branch this conversation"),
    ("/rewind", Slash::Rewind, "Go back to the latest message"),
    (
        "/permission",
        Slash::Permission,
        "Switch full access on or off",
    ),
    (
        "/share",
        Slash::Share,
        "Save this conversation as an HTML page to share",
    ),
    ("/help", Slash::Help, "List these commands"),
];

/// Reads a command from the start of `text`. `None` is an ordinary
/// message, a path included.
pub fn parse(text: &str) -> Option<(Slash, String)> {
    let text = text.trim();
    let (word, rest) = match text.split_once(char::is_whitespace) {
        Some((word, rest)) => (word, rest.trim()),
        None => (text, ""),
    };
    let word = word.to_ascii_lowercase();
    COMMANDS
        .iter()
        .find(|(name, ..)| *name == word)
        .map(|(_, slash, _)| (*slash, rest.to_owned()))
}

/// What follows a `/` typed where a word starts at the end of `text`, the
/// WebView's skill picker trigger: `None` when the text does not end in
/// such a word, or the word is a path (it has another `/` in it).
pub fn typed(text: &str) -> Option<&str> {
    let at = text.rfind('/')?;
    let word = &text[at + 1..];
    let starts_word = text[..at].chars().last().is_none_or(char::is_whitespace);
    (starts_word && !word.contains(char::is_whitespace)).then_some(word)
}

/// `text` without the `/word` it ends in, for what was picked to take its
/// place.
pub fn without_typed(text: &str) -> String {
    match typed(text) {
        Some(word) => text[..text.len() - word.len() - 1].to_owned(),
        None => text.to_owned(),
    }
}

/// The commands whose name begins with what was typed after `/`.
pub fn matching(word: &str) -> Vec<(&'static str, Slash, &'static str)> {
    let word = word.to_ascii_lowercase();
    COMMANDS
        .into_iter()
        .filter(|(name, command, _)| *command != Slash::Help && name[1..].starts_with(&word))
        .collect()
}

/// Whether a command is written out to be finished rather than run when
/// it is picked: it needs words after it.
pub fn takes_words(command: Slash) -> bool {
    matches!(command, Slash::Timer | Slash::Btw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_known_command_at_the_start_is_a_command() {
        assert_eq!(parse(" /review "), Some((Slash::Review, String::new())));
        assert_eq!(parse("/REVIEW"), Some((Slash::Review, String::new())));
        assert_eq!(
            parse("/btw  why did the fit fail?\nand when"),
            Some((Slash::Btw, "why did the fit fail?\nand when".into()))
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
    fn a_slash_where_a_word_starts_opens_the_picker_on_what_follows() {
        assert_eq!(typed("/"), Some(""));
        assert_eq!(typed("please /rev"), Some("rev"));
        assert_eq!(typed("line one\n/sk"), Some("sk"));
        // A path, a fraction, a slash inside a word, or a finished word is
        // not a trigger.
        for text in ["data/reads", "/data/reads", "1/2", "and/or", "/review ", ""] {
            assert_eq!(typed(text), None, "{text}");
        }
        assert_eq!(without_typed("please /rev"), "please ");
        assert_eq!(without_typed("/"), "");
        assert_eq!(without_typed("no trigger"), "no trigger");
    }

    #[test]
    fn the_menu_narrows_to_commands_that_begin_with_the_word() {
        let names = |word: &str| {
            matching(word)
                .iter()
                .map(|(name, ..)| *name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names("re"), ["/review", "/remember", "/rewind"]);
        assert_eq!(names("TIM"), ["/timer"]);
        assert!(names("zzz").is_empty());
        // Everything but help, which is what the menu is.
        assert_eq!(matching("").len(), COMMANDS.len() - 1);
        assert!(takes_words(Slash::Timer) && takes_words(Slash::Btw));
        assert!(!takes_words(Slash::Review));
    }

    #[test]
    fn every_command_is_listed_once_and_starts_with_a_slash() {
        for (index, (name, slash, about)) in COMMANDS.iter().enumerate() {
            assert!(name.starts_with('/') && !about.is_empty());
            assert_eq!(parse(name), Some((*slash, String::new())));
            assert!(
                COMMANDS[index + 1..]
                    .iter()
                    .all(|(other, ..)| other != name)
            );
        }
    }
}

/// What a finished compaction says.
pub fn compacted_note(before: usize, after: usize, summarised: bool) -> String {
    use crate::i18n::{fill, tr};
    if after >= before {
        return tr("Nothing could be shortened yet; the whole history is archived in the project.")
            .to_owned();
    }
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
        assert_eq!(
            super::compacted_note(11_086, 11_086, false),
            "Nothing could be shortened yet; the whole history is archived in the project."
        );
    }
}

/// What `/timer` was asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimerAsk {
    /// Send `prompt` into the conversation every `secs` seconds.
    Every { secs: i64, prompt: String },
    /// Cancel the conversation's timers.
    Off,
}

/// Reads what follows `/timer`: `off`, or an interval (`10m`, `2h`,
/// `1d`) and the prompt to repeat.
pub fn timer(rest: &str) -> Result<TimerAsk, &'static str> {
    use crate::i18n::tr;
    let rest = rest.trim();
    if matches!(
        rest.to_ascii_lowercase().as_str(),
        "off" | "stop" | "cancel"
    ) {
        return Ok(TimerAsk::Off);
    }
    let usage = tr("Write /timer 30m <prompt> (m, h or d), or /timer off.");
    let (interval, prompt) = rest.split_once(char::is_whitespace).ok_or(usage)?;
    let digits = interval.trim_end_matches(|ch: char| ch.is_ascii_alphabetic());
    let unit = match interval[digits.len()..].to_ascii_lowercase().as_str() {
        "m" | "min" => 60,
        "h" => 3_600,
        "d" => 86_400,
        _ => return Err(usage),
    };
    let count: i64 = digits.parse().map_err(|_| usage)?;
    let secs = count.checked_mul(unit).ok_or(usage)?;
    if !(60..=366 * 86_400).contains(&secs) {
        return Err(tr(
            "A timer repeats at least every minute and at most once a year.",
        ));
    }
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err(usage);
    }
    Ok(TimerAsk::Every {
        secs,
        prompt: prompt.to_owned(),
    })
}

#[cfg(test)]
mod timer_tests {
    use super::*;

    #[test]
    fn a_timer_is_an_interval_and_a_prompt_or_off() {
        assert_eq!(
            timer(" 30m  check the queue\nand report "),
            Ok(TimerAsk::Every {
                secs: 1_800,
                prompt: "check the queue\nand report".into()
            })
        );
        assert_eq!(
            timer("2H look"),
            Ok(TimerAsk::Every {
                secs: 7_200,
                prompt: "look".into()
            })
        );
        for off in ["off", " OFF ", "stop", "cancel"] {
            assert_eq!(timer(off), Ok(TimerAsk::Off));
        }
        for wrong in [
            "",
            "30m",
            "30m  ",
            "soon check",
            "30s check",
            "0m check",
            "m check",
            "400d check",
            "30x check",
        ] {
            assert!(timer(wrong).is_err(), "{wrong}");
        }
    }
}
