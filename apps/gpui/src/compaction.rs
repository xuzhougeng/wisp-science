//! Compaction in the transcript: the quiet line where the conversation was
//! compacted, read from the `compaction` row the host puts after the last
//! message saved before it, and the kinds of compaction the compact dialog
//! offers, with the WebView's words for each.
use wisp_dto::{ContextCompactionDto, native_conversations::Item};
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

#[cfg(test)]
mod tests {
    use super::*;

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
