//! Handing the open conversation off to a new one, as the WebView's
//! handoff does: the model writes a note from what it is sent now, the
//! user reads and changes it, and a new conversation starts with the note
//! as its first message. Where the dialog stands and what it allows at
//! each step; the dialog itself is drawn in `workspace::handoff_view`.
use crate::i18n::tr;

/// Where the dialog stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Step {
    /// The model is writing the note.
    #[default]
    Drafting,
    /// The note is in its field, to be read and changed. After a failure
    /// the field keeps what it had, to be written by hand or drafted again.
    Editing,
    /// The new conversation is being started.
    Starting,
}

/// The dialog, for one conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dialog {
    /// The conversation being handed off.
    pub session: String,
    pub step: Step,
    /// Counts the drafts asked for, so a draft that arrives after a newer
    /// one was asked for, or after the dialog closed, fills nothing.
    ticket: u64,
}

impl Dialog {
    /// Opened on `session`; the first draft is asked for at once, as in
    /// the WebView.
    pub fn open(session: String) -> (Self, u64) {
        let dialog = Self {
            session,
            step: Step::Drafting,
            ticket: 1,
        };
        (dialog, 1)
    }

    /// Asks for another draft; `None` while one is being written or the
    /// conversation is being started.
    pub fn redraft(&mut self) -> Option<u64> {
        (self.step == Step::Editing).then(|| {
            self.ticket += 1;
            self.step = Step::Drafting;
            self.ticket
        })
    }

    /// A draft arrived: whether it is the one waited for, which then goes
    /// in the field (or its failure is said) and the note can be edited.
    pub fn drafted(&mut self, ticket: u64) -> bool {
        let current = self.step == Step::Drafting && ticket == self.ticket;
        if current {
            self.step = Step::Editing;
        }
        current
    }

    /// Starts the new conversation from `note`; false when that cannot be
    /// asked now.
    pub fn start(&mut self, note: &str) -> bool {
        let ready = self.can_start(note);
        if ready {
            self.step = Step::Starting;
        }
        ready
    }

    /// Starting failed: the note is still there to try again.
    pub fn start_failed(&mut self) {
        if self.step == Step::Starting {
            self.step = Step::Editing;
        }
    }

    /// The fields and Draft it again take input only while the note is
    /// being read.
    pub fn editable(&self) -> bool {
        self.step == Step::Editing
    }

    /// Start the new conversation, once there is a note.
    pub fn can_start(&self, note: &str) -> bool {
        self.step == Step::Editing && !note.trim().is_empty()
    }

    /// The line that says what the dialog waits for.
    pub fn progress(&self) -> Option<&'static str> {
        match self.step {
            Step::Drafting => Some(tr("Generating the handoff note from the current context…")),
            Step::Starting => Some(tr("Starting the new conversation…")),
            Step::Editing => None,
        }
    }
}

/// Why the open conversation cannot be handed off now, when it cannot: a
/// turn runs in it, it is read-only, or an ACP agent runs it (the host
/// refuses the same).
pub fn refusal(running: bool, read_only: bool, acp: bool) -> Option<&'static str> {
    if read_only {
        Some(tr(
            "This conversation is read-only; it cannot be handed off",
        ))
    } else if acp {
        Some(tr(
            "A conversation an ACP agent runs cannot write a handoff note",
        ))
    } else if running {
        Some(tr(
            "Wait for the current turn to finish before writing a handoff note",
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_note_is_drafted_at_once_then_read_and_changed_before_starting() {
        let (mut dialog, first) = Dialog::open("s".into());
        assert_eq!(dialog.step, Step::Drafting);
        assert!(dialog.progress().is_some());
        // While it is written nothing else can be asked.
        assert!(!dialog.editable());
        assert_eq!(dialog.redraft(), None);
        assert!(!dialog.start("a note"));
        assert!(dialog.drafted(first));
        assert_eq!(dialog.step, Step::Editing);
        assert_eq!(dialog.progress(), None);
        assert!(dialog.editable());
        // An empty note starts nothing.
        assert!(!dialog.can_start("  \n"));
        assert!(!dialog.start(""));
        assert!(dialog.start("## Goal\nAlign the reads."));
        assert_eq!(dialog.step, Step::Starting);
        assert!(!dialog.editable() && dialog.progress().is_some());
        assert_eq!(dialog.redraft(), None);
        // A failure leaves the note to try again.
        dialog.start_failed();
        assert_eq!(dialog.step, Step::Editing);
        assert!(dialog.can_start("note"));
    }

    #[test]
    fn only_the_latest_draft_fills_the_field() {
        let (mut dialog, first) = Dialog::open("s".into());
        assert!(dialog.drafted(first));
        let second = dialog.redraft().unwrap();
        assert_ne!(first, second);
        // The first draft arriving late is not the one waited for.
        assert!(!dialog.drafted(first));
        assert_eq!(dialog.step, Step::Drafting);
        assert!(dialog.drafted(second));
        // Nor is the same one twice.
        assert!(!dialog.drafted(second));
    }

    #[test]
    fn a_conversation_is_handed_off_only_when_idle_writable_and_wisps() {
        assert_eq!(refusal(false, false, false), None);
        for (running, read_only, acp) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
            (true, true, true),
        ] {
            assert!(refusal(running, read_only, acp).is_some());
        }
        // A read-only conversation says so first, whatever else holds.
        assert_eq!(
            refusal(true, true, false),
            Some("This conversation is read-only; it cannot be handed off")
        );
    }
}
