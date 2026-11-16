//! Messages written while a turn runs. The host keeps them for their
//! conversation and sends them one at a time as turns end, whether or not
//! the conversation is open; this says how the strip above the composer
//! words them.
use crate::i18n::tr;
use wisp_server::desktop::QueuedMessage;

/// Why the queue is not moving although no turn runs, in words for the
/// user. `failed` is the last turn ending in an error; `asking` is a
/// question or a plan that waits for the user's own answer.
pub fn hold(failed: bool, asking: bool) -> Option<&'static str> {
    if asking {
        // A queued message must not be taken for the answer.
        Some(tr("Waiting for your answer above"))
    } else if failed {
        Some(tr("Held because the last turn failed"))
    } else {
        None
    }
}

/// Whether a waiting message can be given to the running turn now: only
/// while one runs, and only words, since attachments go with a turn.
pub fn guidable(queued: &QueuedMessage, running: bool) -> bool {
    running && queued.attachments.is_empty()
}

/// One line for a queued message in the strip above the composer.
pub fn label(queued: &QueuedMessage) -> String {
    let first = queued.message.lines().next().unwrap_or_default().trim();
    let attached = queued.attachments.len();
    match (first.is_empty(), attached) {
        (true, n) => crate::files::count(n),
        (false, 0) => first.to_owned(),
        (false, n) => format!("{first} · {}", crate::files::count(n)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued(text: &str, attachments: &[&str]) -> QueuedMessage {
        QueuedMessage {
            id: 1,
            message: text.into(),
            attachments: attachments.iter().map(|path| (*path).into()).collect(),
            references: vec![],
        }
    }

    #[test]
    fn the_queue_holds_for_answers_and_failures() {
        assert_eq!(hold(false, false), None);
        assert!(hold(true, false).unwrap().contains("failed"));
        // The question matters more: answering it is what unblocks.
        assert!(hold(true, true).unwrap().contains("answer"));
    }

    #[test]
    fn a_waiting_message_reads_as_its_first_line_and_files() {
        assert_eq!(label(&queued("first line\nsecond", &[])), "first line");
        assert!(label(&queued("and this", &["data/qc.csv"])).starts_with("and this · "));
        // Words can go into a running turn; nothing can when none runs,
        // and a message with a file waits for a turn of its own.
        assert!(guidable(&queued("use GRCh38", &[]), true));
        assert!(!guidable(&queued("use GRCh38", &[]), false));
        assert!(!guidable(&queued("and this", &["data/qc.csv"]), true));
    }
}
