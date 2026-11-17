//! A model request the running turn sends again after it failed, as the
//! line above the composer says it, in the WebView's words. The host keeps
//! the state; this only words it.
use crate::i18n::{fill, tr};
use wisp_dto::native_conversations::{
    StreamRecoveryCause, StreamRecoveryPhase, StreamRecoveryState,
};

/// The line's title, what happens meanwhile, and the failure being
/// recovered from when the provider said one.
pub fn lines(state: &StreamRecoveryState) -> (String, String, Option<String>) {
    let (attempt, max) = (state.attempt, state.max_attempts);
    let (title, note) = match state.phase {
        StreamRecoveryPhase::Reconnecting => (
            tr("Reconnecting to the model…").to_owned(),
            fill(
                tr("Rebuilding the request from the confirmed context (attempt {}/{})"),
                &[&attempt, &max],
            ),
        ),
        // Waiting, or a phase this client does not know: the wait it
        // reported is what there is to say.
        _ => (
            fill(
                match state.cause {
                    StreamRecoveryCause::Request => tr("Model request failed; retrying ({}/{})"),
                    _ => tr("Model response stream interrupted; recovering the connection ({}/{})"),
                },
                &[&attempt, &max],
            ),
            fill(
                tr(
                    "Retrying in {} s. Completed tool results are kept; the unfinished reply was discarded. Stop ends the turn now.",
                ),
                &[&state.retry_in_ms.div_ceil(1_000)],
            ),
        ),
    };
    let error = (!state.error.trim().is_empty())
        .then(|| fill(tr("Last error: {}"), &[&state.error.trim()]));
    (title, note, error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(phase: StreamRecoveryPhase, cause: StreamRecoveryCause) -> StreamRecoveryState {
        StreamRecoveryState {
            phase,
            cause,
            attempt: 2,
            max_attempts: 5,
            retry_in_ms: 2_500,
            error: "stream ended without completion".into(),
        }
    }

    #[test]
    fn a_cut_stream_says_it_is_recovering_and_when_it_tries_again() {
        let (title, note, error) = lines(&state(
            StreamRecoveryPhase::Waiting,
            StreamRecoveryCause::Stream,
        ));
        assert_eq!(
            title,
            "Model response stream interrupted; recovering the connection (2/5)"
        );
        // Part of a second is a whole one to wait.
        assert!(note.starts_with("Retrying in 3 s."), "{note}");
        assert_eq!(
            error.as_deref(),
            Some("Last error: stream ended without completion")
        );
    }

    #[test]
    fn a_refused_request_and_a_reconnect_say_so_in_their_own_words() {
        let (title, ..) = lines(&state(
            StreamRecoveryPhase::Waiting,
            StreamRecoveryCause::Request,
        ));
        assert_eq!(title, "Model request failed; retrying (2/5)");
        let (title, note, _) = lines(&state(
            StreamRecoveryPhase::Reconnecting,
            StreamRecoveryCause::Stream,
        ));
        assert_eq!(title, "Reconnecting to the model…");
        assert_eq!(
            note,
            "Rebuilding the request from the confirmed context (attempt 2/5)"
        );
        // A phase or cause this client does not know reads as a wait.
        let (title, ..) = lines(&state(
            StreamRecoveryPhase::Other,
            StreamRecoveryCause::Other,
        ));
        assert!(title.starts_with("Model response stream interrupted"));
        // No error, no line for it.
        let quiet = StreamRecoveryState {
            error: "  ".into(),
            ..state(StreamRecoveryPhase::Waiting, StreamRecoveryCause::Stream)
        };
        assert_eq!(lines(&quiet).2, None);
    }
}
