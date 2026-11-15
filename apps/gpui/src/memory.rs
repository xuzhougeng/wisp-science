//! Wording and small rules for memory: the project's notes the model can
//! search, and the habits it is shown in every project. No graphics state.
use crate::i18n::tr;
use wisp_server::desktop::MemoryView;

/// Characters one remembered entry or habit may hold; the host enforces it.
pub const LIMIT: usize = 4_000;

/// What the memory editor is writing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Text taken from a message of this conversation, to be kept either for
    /// the project or as a habit; `turn` is the turn it is from, which the
    /// model can draft a memory of.
    Remember {
        session: Option<String>,
        turn: Option<wisp_server::desktop::TurnRef>,
    },
    /// A habit: a new one, or the one with this id.
    Habit(Option<String>),
    /// One of the project's notes, by file name.
    Note(String),
}

impl Target {
    pub fn title(&self) -> String {
        match self {
            Target::Remember { .. } => tr("Save to memory").into(),
            Target::Habit(None) => tr("New habit").into(),
            Target::Habit(Some(_)) => tr("Edit habit").into(),
            Target::Note(name) => crate::i18n::fill(tr("Note {}"), &[&note_day(name)]),
        }
    }

    /// What the editor says under the text.
    pub fn hint(&self) -> &'static str {
        match self {
            Target::Remember { .. } => tr(
                "Trim it to what is worth keeping. For this project it joins today's note, \
                 which the model can search; as a habit it is shown to the model in every project.",
            ),
            Target::Habit(_) => tr(
                "How you like to work, in a sentence or two. The model is shown your habits \
                 at the start of every turn, in every project.",
            ),
            Target::Note(_) => tr(
                "The whole note for that day. Entries are apart by an empty line; the model \
                 finds them by searching.",
            ),
        }
    }

    /// Whether the entry is held to [`LIMIT`]. A note is a whole file.
    fn limited(&self) -> bool {
        !matches!(self, Target::Note(_))
    }
}

/// What the editor says of where the model suggested keeping its draft.
pub fn suggested_scope(scope: &str) -> &'static str {
    if scope == "global" {
        tr("The model suggests keeping it as a habit.")
    } else {
        tr("The model suggests keeping it for this project.")
    }
}

/// A note's file name without its extension: the day, for notes Wisp wrote.
pub fn note_day(name: &str) -> &str {
    name.strip_suffix(".md").unwrap_or(name)
}

/// A note as the list names it.
pub fn note_label(name: &str, today: &str) -> String {
    if name == today {
        crate::i18n::fill(tr("Today · {}"), &[&note_day(name)])
    } else {
        note_day(name).to_owned()
    }
}

/// The start of some text on one line, for a list.
pub fn preview(text: &str, limit: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= limit {
        return line;
    }
    let mut short: String = line.chars().take(limit - 1).collect();
    short.truncate(short.trim_end().len());
    short.push('…');
    short
}

/// A message's text as it is offered for remembering: no more than one
/// entry may hold, so the editor never opens on something unsavable.
pub fn offered(text: &str) -> String {
    text.trim().chars().take(LIMIT).collect()
}

/// Why the editor's text cannot be saved as it is, if it cannot.
pub fn problem(target: &Target, text: &str) -> Option<String> {
    let length = text.trim().chars().count();
    if length == 0 {
        return Some(tr("Write something first.").into());
    }
    (target.limited() && length > LIMIT).then(|| {
        crate::i18n::fill(
            tr("{} characters; one entry holds at most {}."),
            &[&length, &LIMIT],
        )
    })
}

/// One line under the switch saying what there is.
pub fn summary(view: &MemoryView) -> String {
    let notes = match view.notes.len() {
        0 => tr("no notes").to_owned(),
        1 => tr("1 note").to_owned(),
        n => crate::i18n::fill(tr("{} notes"), &[&n]),
    };
    let habits = match view.habits.len() {
        0 => tr("no habits").to_owned(),
        n => habits(n),
    };
    crate::i18n::fill(tr("{} here, {} everywhere."), &[&notes, &habits])
}

/// How many habits, in words: 1 habit, 3 habits.
pub fn habits(count: usize) -> String {
    match count {
        1 => tr("1 habit").into(),
        n => crate::i18n::fill(tr("{} habits"), &[&n]),
    }
}

/// What a tidy proposal does, in words.
pub fn tidy_action(action: &str) -> &'static str {
    match action {
        "merge" => tr("Merge"),
        _ => tr("Retire"),
    }
}

/// How the weekly tidy last went.
pub fn tidy_line<Tz: chrono::TimeZone>(tidy: &wisp_dto::MemoryTidyAutomation, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    use crate::i18n::fill;
    if tidy.running {
        return tr("Looking over the habits…").to_owned();
    }
    let Some(at) = tidy.last_run_at else {
        return tr("Not run yet").to_owned();
    };
    let when = zone
        .timestamp_opt(at, 0)
        .earliest()
        .map(|time| time.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    match &tidy.error {
        Some(error) => fill(
            tr("Failed {}: {}"),
            &[&when, &error.lines().next().unwrap_or_default()],
        ),
        None if tidy.proposals.is_empty() => {
            fill(tr("Looked over on {}: nothing to change"), &[&when])
        }
        None => fill(
            tr("Looked over on {}: {} to review"),
            &[&when, &tidy.proposals.len()],
        ),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_tidy_says_how_it_last_went() {
        use chrono::{FixedOffset, TimeZone};
        let zone = FixedOffset::east_opt(8 * 3_600).unwrap();
        let mut tidy = wisp_dto::MemoryTidyAutomation::default();
        assert_eq!(tidy_line(&tidy, &zone), "Not run yet");
        tidy.last_run_at = Some(
            zone.with_ymd_and_hms(2026, 11, 15, 9, 0, 0)
                .unwrap()
                .timestamp(),
        );
        assert_eq!(
            tidy_line(&tidy, &zone),
            "Looked over on 2026-11-15: nothing to change"
        );
        tidy.proposals.push(wisp_dto::MemoryTidyProposal {
            id: "p".into(),
            action: "merge".into(),
            memories: vec![],
            text: "x".into(),
            reason: String::new(),
        });
        assert_eq!(
            tidy_line(&tidy, &zone),
            "Looked over on 2026-11-15: 1 to review"
        );
        tidy.error = Some("model unreachable".into());
        assert_eq!(
            tidy_line(&tidy, &zone),
            "Failed 2026-11-15: model unreachable"
        );
        tidy.running = true;
        assert_eq!(tidy_line(&tidy, &zone), "Looking over the habits…");
        assert_eq!(tidy_action("merge"), "Merge");
        assert_eq!(tidy_action("retire"), "Retire");
    }

    use super::*;
    use wisp_server::desktop::{GlobalMemory, MemoryNote};

    fn view(notes: usize, habits: usize) -> MemoryView {
        MemoryView {
            enabled: true,
            today: "2026-11-14.md".into(),
            notes: (0..notes)
                .map(|n| MemoryNote {
                    name: format!("2026-11-{:02}.md", 14 - n),
                    preview: String::new(),
                    bytes: 0,
                })
                .collect(),
            habits: (0..habits)
                .map(|n| GlobalMemory {
                    id: n.to_string(),
                    content: String::new(),
                    source_frame_id: None,
                    source_turn_index: None,
                    created_at: 0,
                    updated_at: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn notes_are_named_by_their_day_and_today_is_marked() {
        assert_eq!(
            note_label("2026-11-14.md", "2026-11-14.md"),
            "Today · 2026-11-14"
        );
        assert_eq!(note_label("2026-11-01.md", "2026-11-14.md"), "2026-11-01");
        // A file someone put there by hand keeps its name.
        assert_eq!(note_label("protocols.md", "2026-11-14.md"), "protocols");
        assert_eq!(note_label("README", "2026-11-14.md"), "README");
        assert_eq!(
            Target::Note("2026-11-01.md".into()).title(),
            "Note 2026-11-01"
        );
    }

    #[test]
    fn the_summary_counts_what_is_here_and_everywhere() {
        assert_eq!(summary(&view(0, 0)), "no notes here, no habits everywhere.");
        assert_eq!(summary(&view(1, 1)), "1 note here, 1 habit everywhere.");
        assert_eq!(summary(&view(3, 2)), "3 notes here, 2 habits everywhere.");
    }

    #[test]
    fn the_suggested_place_is_said_in_words() {
        assert_eq!(
            suggested_scope("global"),
            "The model suggests keeping it as a habit."
        );
        assert_eq!(
            suggested_scope("project"),
            "The model suggests keeping it for this project."
        );
    }

    #[test]
    fn a_preview_is_one_line_cut_on_characters() {
        assert_eq!(preview("  one\n\ntwo   three ", 40), "one two three");
        let long = preview(&"基因 ".repeat(50), 20);
        assert_eq!(long.chars().count(), 20);
        assert!(long.ends_with('…') && !long.ends_with(" …"));
    }

    #[test]
    fn an_entry_is_held_to_the_limit_and_a_note_is_not() {
        let long = "x".repeat(LIMIT + 5);
        assert_eq!(offered(&format!("  {long}  ")).chars().count(), LIMIT);
        let remember = Target::Remember {
            session: None,
            turn: None,
        };
        assert_eq!(
            problem(&remember, "  "),
            Some("Write something first.".into())
        );
        assert_eq!(problem(&remember, "keep this"), None);
        assert!(
            problem(&remember, &long)
                .unwrap()
                .contains("4005 characters")
        );
        assert!(problem(&Target::Habit(None), &long).is_some());
        assert_eq!(problem(&Target::Note("a.md".into()), &long), None);
        assert!(problem(&Target::Note("a.md".into()), "\n").is_some());
    }
}
