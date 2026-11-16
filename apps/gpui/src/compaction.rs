//! Compaction in the transcript: the quiet line where the conversation was
//! compacted, read from the `compaction` row the host puts after the last
//! message saved before it, and the kinds of compaction the compact dialog
//! offers, with the WebView's words for each. Also the way on from a turn
//! the model refused because the conversation outgrew its context window:
//! the WebView's three choices, offered in a bar above the composer.
use wisp_dto::{
    ContextCompactionDto,
    native_conversations::{Item, Snapshot, TurnErrorKind},
};
use wisp_server::desktop::CompactKind;

use crate::{
    controls::compact,
    i18n::{fill, tr},
};

/// The compaction a transcript row stands for, when it is one.
pub fn card(row: &Item) -> Option<ContextCompactionDto> {
    (row.role == "compaction")
        .then(|| serde_json::from_str(&row.text).ok())
        .flatten()
}

/// The line the row says: what happened, how far it went, and how much
/// smaller the context became.
pub fn caption(card: &ContextCompactionDto) -> String {
    let what = if card.strategy == "manual" {
        tr("Context compacted")
    } else {
        tr("Context automatically compacted")
    };
    // Only a summary leaves a checkpoint behind.
    let how = if card.checkpoint.is_some() {
        tr("older turns summarised")
    } else {
        tr("old tool output trimmed")
    };
    let mut line = format!(
        "{what} · {how} · {}",
        fill(
            tr("{} → {} tokens"),
            &[&compact(card.before as u64), &compact(card.after as u64)]
        )
    );
    if let Some(percent) = reduction(card) {
        line.push_str(" · ");
        line.push_str(&fill(tr("{}% smaller"), &[&percent]));
    }
    line
}

/// How much smaller the context became, in whole percent; `None` when it
/// did not shrink.
pub fn reduction(card: &ContextCompactionDto) -> Option<usize> {
    (card.before > card.after && card.before > 0)
        .then(|| ((card.before - card.after) as f64 * 100.0 / card.before as f64).round() as usize)
}

/// Whether the row offers Undo: the host says the compaction can still be
/// undone (it is the latest, nothing was said after it, the conversation
/// is not archived) and no turn is running or the conversation is only
/// watched.
pub fn undo_offered(card: &ContextCompactionDto, running: bool, read_only: bool) -> bool {
    card.can_undo && card.epoch > 0 && !running && !read_only
}

/// A kind of compaction the dialog offers: the ones the host can do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Regular,
    Semantic,
}

impl Kind {
    /// In the order the dialog lists them.
    pub const ALL: [Kind; 2] = [Kind::Regular, Kind::Semantic];

    pub fn title(self) -> &'static str {
        match self {
            Kind::Regular => tr("Regular compact"),
            Kind::Semantic => tr("Semantic compact"),
        }
    }

    /// What it does, in the WebView's words.
    pub fn hint(self) -> &'static str {
        match self {
            Kind::Regular => tr(
                "Archive the transcript, then replace old tool results with short stubs. User and assistant turns stay in place.",
            ),
            Kind::Semantic => tr("Fold older turns into a checkpoint plus a short retained tail."),
        }
    }

    /// What the host is asked for.
    pub fn host(self) -> CompactKind {
        match self {
            Kind::Regular => CompactKind::Regular,
            Kind::Semantic => CompactKind::Semantic,
        }
    }
}

/// A way on from a turn the model refused as too long, the WebView's three.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// The compact dialog, set to semantic; once the compaction is done the
    /// unfinished turn goes on from what was saved.
    CompactAndContinue,
    /// A new conversation that reads this one and goes on with the request.
    NewConversation,
    /// Nothing more is sent; the error and the work stay.
    Pause,
}

impl Recovery {
    /// In the order the bar offers them.
    pub const ALL: [Recovery; 3] = [
        Recovery::CompactAndContinue,
        Recovery::NewConversation,
        Recovery::Pause,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Recovery::CompactAndContinue => tr("Compact and continue"),
            Recovery::NewConversation => tr("Continue in a new conversation"),
            Recovery::Pause => tr("Pause conversation"),
        }
    }

    /// What it does, in the WebView's words but for what this desktop
    /// does: the new conversation reads an excerpt of this one, not a
    /// Reader's summary.
    pub fn hint(self) -> &'static str {
        match self {
            Recovery::CompactAndContinue => tr(
                "Archive the full history, fold older turns, then retry the unfinished response without replaying completed tools.",
            ),
            Recovery::NewConversation => tr(
                "Start fresh in this project with this conversation attached as context, and go on with the unfinished request.",
            ),
            Recovery::Pause => {
                tr("Keep the error and all completed work here without sending another request.")
            }
        }
    }
}

/// What the new conversation is asked first, in the WebView's words.
pub fn continue_prompt() -> &'static str {
    tr(
        "Continue the last unfinished request from the attached conversation. Use its summarized context, preserve completed work, and report the final result, verification, and anything still incomplete.",
    )
}

/// The failure the bar is about, so pausing on it does not hide a later
/// one: the conversation and the turn that failed.
pub fn failure(snapshot: &Snapshot) -> String {
    format!(
        "{}\n{}",
        snapshot.session_id,
        snapshot.request_id.as_deref().unwrap_or_default()
    )
}

/// Whether the bar shows: the last turn was refused as too long, nothing
/// runs, the conversation can be written to, Wisp runs it (an ACP agent
/// keeps its own context) and the user has not paused on this failure.
pub fn recovery_offered(snapshot: &Snapshot, read_only: bool, paused: bool) -> bool {
    snapshot.error_kind == Some(TurnErrorKind::ContextTooLong)
        && !snapshot.running
        && !read_only
        && snapshot.acp_agent_id.is_none()
        && !paused
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed(kind: Option<&str>) -> Snapshot {
        serde_json::from_value(serde_json::json!({
            "schema": "v1", "epoch": "e", "sequence": 1, "project_id": "p",
            "session_id": "s", "items": [], "next_before_seq": null,
            "running": false, "stopping": false, "read_only": false,
            "model_id": "m", "request_id": "r1",
            "error": "api: 400 This model's maximum context length is 8192 tokens.",
            "error_kind": kind, "approvals": []
        }))
        .unwrap()
    }

    #[test]
    fn the_bar_shows_for_a_turn_refused_as_too_long_until_paused() {
        let too_long = failed(Some("context_too_long"));
        assert!(recovery_offered(&too_long, false, false));
        // Not while a turn runs, in a conversation that cannot be written
        // to, in one an ACP agent runs, or once paused on.
        let mut running = too_long.clone();
        running.running = true;
        assert!(!recovery_offered(&running, false, false));
        assert!(!recovery_offered(&too_long, true, false));
        let mut acp = too_long.clone();
        acp.acp_agent_id = Some("agent".into());
        assert!(!recovery_offered(&acp, false, false));
        assert!(!recovery_offered(&too_long, false, true));
        // Any other failure, one an older host does not name, or one this
        // build does not know: the error alone.
        assert!(!recovery_offered(&failed(None), false, false));
        assert!(!recovery_offered(&failed(Some("quota")), false, false));
        // Pausing is on this failure: the next turn that fails is another.
        let mut later = too_long.clone();
        later.request_id = Some("r2".into());
        assert_ne!(failure(&too_long), failure(&later));
        assert_eq!(failure(&too_long), failure(&too_long.clone()));
    }

    #[test]
    fn the_bar_offers_the_webviews_three_ways_on() {
        assert_eq!(
            Recovery::ALL.map(Recovery::label),
            [
                "Compact and continue",
                "Continue in a new conversation",
                "Pause conversation"
            ]
        );
        for recovery in Recovery::ALL {
            assert!(!recovery.hint().is_empty());
        }
        assert!(continue_prompt().starts_with("Continue the last unfinished request"));
    }

    fn row(role: &str, text: &str) -> Item {
        serde_json::from_value(serde_json::json!({
            "role": role, "text": text, "tool_name": null,
            "input": null, "ok": null, "status": null
        }))
        .unwrap()
    }

    fn compaction(
        strategy: &str,
        checkpoint: Option<&str>,
        can_undo: bool,
    ) -> ContextCompactionDto {
        ContextCompactionDto {
            epoch: 2,
            before: 90_000,
            after: 12_000,
            strategy: strategy.into(),
            checkpoint: checkpoint.map(str::to_owned),
            kept_from_user_index: None,
            can_undo,
            undo_reason: (!can_undo).then(|| "has_new_turns".into()),
        }
    }

    #[test]
    fn a_compaction_row_is_read_and_other_rows_are_not() {
        let written = compaction("manual", None, true);
        let text = serde_json::to_string(&written).unwrap();
        let found = card(&row("compaction", &text)).unwrap();
        assert_eq!(
            (found.epoch, found.before, found.after),
            (2, 90_000, 12_000)
        );
        assert!(found.can_undo);
        // Another role with the same text, or a row the host garbled, is not.
        assert!(card(&row("assistant", &text)).is_none());
        assert!(card(&row("compaction", "not json")).is_none());
    }

    #[test]
    fn the_line_says_who_compacted_how_far_and_how_much() {
        let asked = caption(&compaction("manual", None, true));
        assert!(
            asked.starts_with("Context compacted · old tool output trimmed"),
            "{asked}"
        );
        assert!(asked.contains(" · 90.0k → 12.0k tokens · "), "{asked}");
        assert!(asked.ends_with("87% smaller"), "{asked}");
        let by_itself = caption(&compaction(
            "auto",
            Some("[context summary checkpoint] …"),
            false,
        ));
        assert!(
            by_itself.starts_with("Context automatically compacted · older turns summarised"),
            "{by_itself}"
        );
        // A context that overflowed and was compacted to go on is automatic too.
        assert!(caption(&compaction("overflow", None, false)).starts_with("Context automatically"));
        // Nothing gained, nothing claimed.
        let mut same = compaction("manual", None, true);
        same.after = same.before;
        assert_eq!(reduction(&same), None);
        assert!(!caption(&same).contains("smaller"));
        same.before = 0;
        same.after = 0;
        assert_eq!(reduction(&same), None);
    }

    #[test]
    fn undo_is_offered_only_while_the_host_allows_it_and_nothing_runs() {
        let latest = compaction("manual", None, true);
        assert!(undo_offered(&latest, false, false));
        assert!(!undo_offered(&latest, true, false));
        assert!(!undo_offered(&latest, false, true));
        // Once the conversation went on, the line only says what happened.
        assert!(!undo_offered(
            &compaction("manual", None, false),
            false,
            false
        ));
        // A compaction the host does not name cannot be asked for.
        let mut unnamed = latest.clone();
        unnamed.epoch = 0;
        assert!(!undo_offered(&unnamed, false, false));
    }

    #[test]
    fn the_dialog_offers_the_two_kinds_the_host_does() {
        assert_eq!(Kind::default(), Kind::Regular);
        assert_eq!(Kind::ALL, [Kind::Regular, Kind::Semantic]);
        assert_eq!(Kind::Regular.host(), CompactKind::Regular);
        assert_eq!(Kind::Semantic.host(), CompactKind::Semantic);
        for kind in Kind::ALL {
            assert!(!kind.title().is_empty() && !kind.hint().is_empty());
        }
        assert_ne!(Kind::Regular.hint(), Kind::Semantic.hint());
    }
}
