//! Reusing a range of the open conversation's turns, as the WebView's
//! Quote / reuse does: Quote / reuse on one user message starts the range,
//! End quote here on the same or another one ends it, and a dialog shows
//! the turns. They are either quoted as they are, or distilled by the
//! conversation's model into an experience draft the user edits and may
//! keep as a memory; then the quote goes into the draft of a new
//! conversation or another one of the project, to be sent from there.
//! Which turns are chosen, what is quoted and what is kept; the dialog is
//! drawn in `workspace::reuse_view`.
use crate::i18n::{fill, tr};
use wisp_dto::{SessionExcerpt, SessionSearchInfo, native_conversations::Item};
use wisp_server::desktop::TurnRef;

/// The first turn of a range being chosen, in one conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub session: String,
    /// The user message that starts it, as the host names it.
    pub start: TurnRef,
}

/// The transcript rows of user messages, each with how many user messages
/// follow it.
fn user_rows(rows: &[Item]) -> impl Iterator<Item = (usize, usize)> + '_ {
    let total = rows.iter().filter(|row| row.role == "user").count();
    rows.iter()
        .enumerate()
        .filter(|(_, row)| row.role == "user")
        .enumerate()
        .map(move |(nth, (index, _))| (index, total - 1 - nth))
}

impl Selection {
    /// Starts a range at row `index` of `session`, which must be a user
    /// message: the WebView offers this on the user's messages only.
    pub fn begin(session: &str, rows: &[Item], index: usize) -> Option<Self> {
        if rows.get(index)?.role != "user" {
            return None;
        }
        Some(Self {
            session: session.to_owned(),
            start: crate::controls::turn_of(rows, index)?,
        })
    }

    /// The row the range starts at now. Turns sent since come after it and
    /// earlier pages loaded since come before it, so it is the message with
    /// the same text the fewest user messages from the end, but no fewer
    /// than when it was chosen.
    pub fn row(&self, rows: &[Item]) -> Option<usize> {
        user_rows(rows)
            .filter(|&(index, from_end)| {
                from_end >= self.start.from_end && rows[index].text == self.start.text
            })
            .min_by_key(|&(_, from_end)| from_end)
            .map(|(index, _)| index)
    }

    /// The range from the start to the user message at row `index`, the
    /// earlier turn first, as the host is asked for it.
    pub fn range(&self, rows: &[Item], index: usize) -> Option<(TurnRef, TurnRef)> {
        if rows.get(index)?.role != "user" {
            return None;
        }
        let start = crate::controls::turn_of(rows, self.row(rows)?)?;
        let end = crate::controls::turn_of(rows, index)?;
        Some(if start.from_end >= end.from_end {
            (start, end)
        } else {
            (end, start)
        })
    }

    /// The 1-based number of the starting turn, when the whole
    /// conversation is loaded and so every turn before it is counted.
    pub fn number(&self, rows: &[Item], whole: bool) -> Option<usize> {
        let row = self.row(rows)?;
        whole.then(|| rows[..row].iter().filter(|r| r.role == "user").count() + 1)
    }
}

/// The message action on a user message: Quote / reuse, or End quote
/// here once a range was started; its icon and its name.
pub fn action(selecting: bool) -> (&'static str, &'static str) {
    if selecting {
        ("check-square", tr("End quote here"))
    } else {
        ("list", tr("Quote / reuse"))
    }
}

/// Whether the action is offered on row `index`: a user message of a
/// conversation with saved turns to read, which one an ACP agent runs does
/// not have, and not the turn that is still running, since only finished
/// turns are quoted. Read-only conversations qualify: quoting only reads.
pub fn offered(rows: &[Item], index: usize, acp: bool, running: bool) -> bool {
    let latest = user_rows(rows).find(|&(_, from_end)| from_end == 0);
    !acp && rows.get(index).is_some_and(|row| row.role == "user")
        && !(running && latest.is_some_and(|(row, _)| row == index))
}

/// What the excerpt is called: the conversation's title and the turns,
/// counted from 1 as the transcript counts them.
pub fn source_label(excerpt: &SessionExcerpt) -> String {
    let title = excerpt.title.trim();
    let title = if title.is_empty() {
        tr("Untitled conversation").to_owned()
    } else {
        crate::present::one_line(title)
    };
    let start = excerpt.start_user_index + 1;
    let end = excerpt.end_user_index + 1;
    if start == end {
        fill(tr("{} · turn {}"), &[&title, &start])
    } else {
        fill(tr("{} · turns {}–{}"), &[&title, &start, &end])
    }
}

/// How the turns are reused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// The turns as they were said.
    #[default]
    Original,
    /// What the model distilled from them, as the user edited it.
    Experience,
}

/// What goes in front of the destination's draft, as the WebView sends a
/// quote: the line that names the source, then the turns, or the
/// experience draft, quoted. None for an experience not written yet.
pub fn quote(mode: Mode, excerpt: &SessionExcerpt, draft: &str) -> Option<String> {
    let label = source_label(excerpt);
    match mode {
        Mode::Original => Some(format!(
            "{}{}",
            wisp_dto::quoted_turns_heading(&label),
            wisp_dto::blockquote_lines(&excerpt.text)
        )),
        Mode::Experience if draft.trim().is_empty() => None,
        Mode::Experience => Some(format!(
            "{}{}",
            wisp_dto::distilled_experience_heading(&label),
            wisp_dto::blockquote_lines(draft)
        )),
    }
}

/// The destination's draft once `quote` is put in front of what was being
/// written there.
pub fn with_quote(typed: &str, quote: &str) -> String {
    let typed = typed.trim();
    if typed.is_empty() {
        format!("{quote}\n")
    } else {
        format!("{quote}\n{typed}")
    }
}

/// What is kept when an experience draft is saved as a memory: the draft,
/// and where it came from, as the WebView keeps it.
pub fn memory_text(excerpt: &SessionExcerpt, draft: &str) -> String {
    format!(
        "{}\n\n{}: {} (session {})",
        draft.trim(),
        tr("Source"),
        source_label(excerpt),
        excerpt.session_id
    )
}

/// The conversations of `project` a quote can be put in, newest first as
/// listed, whose title has `query` in it: not the source, not a subagent's
/// (`dispatched`), which takes no messages from the user, and not an
/// archived one, which is read-only.
pub fn destinations<'a>(
    listed: &'a [SessionSearchInfo],
    project: &str,
    source: &str,
    dispatched: impl Fn(&str) -> bool,
    query: &str,
) -> Vec<&'a SessionSearchInfo> {
    let query = query.trim().to_lowercase();
    listed
        .iter()
        .filter(|row| {
            row.project_id == project
                && row.id != source
                && !row.shelved
                && !dispatched(&row.id)
                && (query.is_empty() || row.title.to_lowercase().contains(&query))
        })
        .collect()
}

/// The dialog, for one excerpt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dialog {
    pub excerpt: SessionExcerpt,
    /// The turns, as the host is asked for an experience from them.
    pub first: TurnRef,
    pub last: TurnRef,
    pub mode: Mode,
    /// Where the quote goes: a conversation of the project, or a new one.
    pub destination: Option<String>,
    /// The model is distilling the experience.
    pub generating: bool,
    /// Part of the evidence was cut before the draft was written.
    pub shortened: bool,
    /// The draft as it reads now is kept as a memory.
    pub saved: bool,
    /// A memory is being saved or the quote put in its place.
    pub busy: bool,
    /// Counts the drafts asked for, so only the latest fills the field.
    ticket: u64,
}

impl Dialog {
    pub fn open(excerpt: SessionExcerpt, first: TurnRef, last: TurnRef) -> Self {
        Self {
            excerpt,
            first,
            last,
            mode: Mode::Original,
            destination: None,
            generating: false,
            shortened: false,
            saved: false,
            busy: false,
            ticket: 0,
        }
    }

    /// Asks for a draft; `None` while one is written or the dialog is busy.
    pub fn generate(&mut self) -> Option<u64> {
        if self.generating || self.busy {
            return None;
        }
        self.generating = true;
        self.ticket += 1;
        Some(self.ticket)
    }

    /// A draft arrived: whether it is the one waited for.
    pub fn generated(&mut self, ticket: u64) -> bool {
        let current = self.generating && ticket == self.ticket;
        if current {
            self.generating = false;
        }
        current
    }

    /// The draft changed: what was kept is no longer what it says.
    pub fn edited(&mut self) {
        self.saved = false;
    }

    /// Save as a memory, once there is a draft that is not kept yet.
    pub fn can_save(&self, draft: &str) -> bool {
        self.mode == Mode::Experience
            && !self.generating
            && !self.busy
            && !self.saved
            && !draft.trim().is_empty()
    }

    /// Add to conversation: the turns, or an experience once written.
    pub fn can_attach(&self, draft: &str) -> bool {
        !self.generating && !self.busy && (self.mode == Mode::Original || !draft.trim().is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(role: &str, text: &str) -> Item {
        serde_json::from_value(json!({
            "role": role, "text": text, "tool_name": null, "input": null,
            "ok": null, "status": null
        }))
        .unwrap()
    }

    fn rows() -> Vec<Item> {
        vec![
            item("user", "count the reads"),
            item("assistant", "There are 100."),
            item("compaction", ""),
            item("user", "normalize"),
            item("tool", "ls"),
            item("assistant", "Done."),
            item("user", "plot it"),
            item("assistant", "Plotted."),
        ]
    }

    fn excerpt(title: &str, start: usize, end: usize) -> SessionExcerpt {
        SessionExcerpt {
            session_id: "s".into(),
            title: title.into(),
            start_user_index: start,
            end_user_index: end,
            text: "User:\nq\n\nAssistant:\na".into(),
            truncated: false,
        }
    }

    #[test]
    fn a_range_starts_and_ends_on_user_messages_only() {
        let rows = rows();
        // An answer, a tool call or a compaction line starts nothing.
        for index in [1, 2, 4, 5, 9] {
            assert_eq!(Selection::begin("s", &rows, index), None, "{index}");
        }
        let selection = Selection::begin("s", &rows, 3).unwrap();
        assert_eq!(
            selection.start,
            TurnRef {
                from_end: 1,
                text: "normalize".into()
            }
        );
        assert_eq!(selection.row(&rows), Some(3));
        assert_eq!(selection.range(&rows, 5), None);
        // Ended later or earlier, the earlier turn comes first.
        let later = selection.range(&rows, 6).unwrap();
        assert_eq!((later.0.from_end, later.1.from_end), (1, 0));
        assert_eq!(later.1.text, "plot it");
        let earlier = selection.range(&rows, 0).unwrap();
        assert_eq!((earlier.0.from_end, earlier.1.from_end), (2, 1));
        assert_eq!(earlier.0.text, "count the reads");
        // Ended where it started: that one turn.
        let one = selection.range(&rows, 3).unwrap();
        assert_eq!(one.0, one.1);
        assert_eq!(selection.number(&rows, true), Some(2));
        // With earlier pages not loaded, the number is not known.
        assert_eq!(selection.number(&rows, false), None);
    }

    #[test]
    fn the_start_is_found_again_after_turns_are_sent_or_earlier_pages_load() {
        let mut rows = rows();
        let selection = Selection::begin("s", &rows, 3).unwrap();
        // A turn was sent since: the start is one more from the end.
        rows.push(item("user", "normalize"));
        rows.push(item("assistant", "Again."));
        assert_eq!(selection.row(&rows), Some(3));
        let range = selection.range(&rows, 8).unwrap();
        assert_eq!((range.0.from_end, range.1.from_end), (2, 0));
        // An earlier page came in above it.
        let mut paged = vec![item("user", "hello"), item("assistant", "hi")];
        paged.extend(rows);
        assert_eq!(selection.row(&paged), Some(5));
        // Rewound past it: there is no start any more.
        assert_eq!(selection.row(&paged[..4]), None);
        assert_eq!(selection.range(&paged[..4], 2), None);
    }

    #[test]
    fn the_action_says_whether_it_starts_or_ends_the_range() {
        assert_eq!(action(false), ("list", "Quote / reuse"));
        assert_eq!(action(true), ("check-square", "End quote here"));
    }

    #[test]
    fn finished_turns_are_offered_unless_an_acp_agent_runs_the_conversation() {
        let rows = rows();
        // On the user's messages, whether or not the conversation can be
        // written to: it is only read.
        assert!([0, 3, 6].iter().all(|&i| offered(&rows, i, false, false)));
        assert!(
            [1, 2, 4, 5, 7, 8]
                .iter()
                .all(|&i| !offered(&rows, i, false, false))
        );
        // While a turn runs, every message but the running one.
        assert!(offered(&rows, 3, false, true));
        assert!(!offered(&rows, 6, false, true));
        assert!(!offered(&rows, 0, true, false));
    }

    #[test]
    fn the_quote_names_its_source_and_carries_the_turns_or_the_draft() {
        assert_eq!(source_label(&excerpt("counts", 2, 4)), "counts · turns 3–5");
        assert_eq!(
            source_label(&excerpt("  ", 0, 0)),
            "Untitled conversation · turn 1"
        );
        let turns = excerpt("counts", 2, 4);
        assert_eq!(
            quote(Mode::Original, &turns, "ignored").unwrap(),
            "Quoted turns from an earlier conversation `counts · turns 3–5`:\n\
             > User:\n> q\n> \n> Assistant:\n> a\n"
        );
        assert_eq!(quote(Mode::Experience, &turns, " \n"), None);
        let experience = quote(Mode::Experience, &turns, "  ## Steps\nNormalize first.  ").unwrap();
        assert_eq!(
            experience,
            "Reusable experience distilled from conversation `counts · turns 3–5`:\n\
             > ## Steps\n> Normalize first.\n"
        );
        assert!(!experience.contains("Assistant:"));
        // In front of what the destination's draft already says.
        assert_eq!(with_quote("", "> q\n"), "> q\n\n");
        assert_eq!(with_quote("  why?  \n", "> q\n"), "> q\n\nwhy?");
    }

    #[test]
    fn a_kept_experience_says_where_it_came_from() {
        assert_eq!(
            memory_text(&excerpt("counts", 0, 1), "  Normalize first. \n"),
            "Normalize first.\n\nSource: counts · turns 1–2 (session s)"
        );
    }

    #[test]
    fn a_quote_goes_to_a_writable_conversation_of_the_project_other_than_the_source() {
        let row = |id: &str, project: &str, title: &str, shelved: bool| SessionSearchInfo {
            id: id.into(),
            project_id: project.into(),
            project_name: String::new(),
            title: title.into(),
            ts: 0,
            activity_at: 0,
            status: String::new(),
            shelved,
        };
        let listed = [
            row("s", "p", "Source", false),
            row("a", "p", "QC of counts", false),
            row("b", "p", "Old counts", true),
            row("c", "q", "Counts elsewhere", false),
            row("d", "p", "Subagent counts", false),
            row("e", "p", "Plots", false),
        ];
        let ids = |query: &str| {
            destinations(&listed, "p", "s", |id| id == "d", query)
                .into_iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(""), ["a", "e"]);
        assert_eq!(ids("  COUNTS "), ["a"]);
        assert!(ids("nothing").is_empty());
    }

    #[test]
    fn a_draft_is_asked_for_one_at_a_time_and_only_the_latest_fills_the_field() {
        let turn = TurnRef {
            from_end: 0,
            text: "q".into(),
        };
        let mut dialog = Dialog::open(excerpt("t", 0, 0), turn.clone(), turn);
        assert!(dialog.can_attach(""));
        assert!(!dialog.can_save("draft"));
        dialog.mode = Mode::Experience;
        // An experience is added or kept only once there is one.
        assert!(!dialog.can_attach(" "));
        assert!(!dialog.can_save(" "));
        let first = dialog.generate().unwrap();
        assert_eq!(dialog.generate(), None);
        assert!(!dialog.can_attach("draft") && !dialog.can_save("draft"));
        assert!(dialog.generated(first));
        assert!(!dialog.generated(first));
        assert!(dialog.can_attach("draft") && dialog.can_save("draft"));
        let second = dialog.generate().unwrap();
        // The first one arriving late is not waited for.
        assert!(!dialog.generated(first));
        assert!(dialog.generated(second));
        // Kept, it is not saved twice; changed, it can be kept again.
        dialog.saved = true;
        assert!(!dialog.can_save("draft"));
        dialog.edited();
        assert!(dialog.can_save("draft"));
        dialog.busy = true;
        assert!(dialog.generate().is_none());
        assert!(!dialog.can_attach("draft") && !dialog.can_save("draft"));
    }
}
