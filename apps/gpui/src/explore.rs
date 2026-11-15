//! How explorations read in the window: whether one can start from a
//! conversation, and what a candidate's status and isolation are called.
use crate::i18n::{fill, tr};
use wisp_dto::ExplorationSummary;
use wisp_dto::native_conversations::ExplorationState;

/// Whether an exploration can start from this conversation: not from an
/// exploration or a branch, and only from the source of the round that is
/// open, if one is.
pub fn can_start(state: Option<&ExplorationState>, session: &str, branch: bool, acp: bool) -> bool {
    if branch || acp {
        return false;
    }
    match state {
        None => true,
        Some(state) => {
            state.summary.is_none()
                && state
                    .round_source_frame_id
                    .as_deref()
                    .is_none_or(|source| source == session)
        }
    }
}

pub fn status_label(status: &str) -> &'static str {
    match status {
        "creating" => tr("Creating"),
        "active" => tr("Active"),
        "promoting" => tr("Being kept"),
        "failed" => tr("Failed"),
        _ => tr("Ended"),
    }
}

pub fn isolation_label(summary: &ExplorationSummary) -> &'static str {
    if summary.isolation_is_full() {
        tr("Fully isolated")
    } else {
        tr("Partially isolated: some files are referenced, not copied")
    }
}

/// The name a new candidate is offered.
pub fn default_name(candidates: usize) -> String {
    fill(tr("Exploration {}"), &[&(candidates + 1)])
}

/// The round's candidates of `source`, in the order they were made.
pub fn candidates<'a>(rows: &'a [ExplorationSummary], source: &str) -> Vec<&'a ExplorationSummary> {
    let mut found: Vec<_> = rows
        .iter()
        .filter(|row| row.source_frame_id == source)
        .collect();
    found.sort_by_key(|row| row.exploration.created_at);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, source: &str, at: i64) -> ExplorationSummary {
        serde_json::from_value(serde_json::json!({
            "exploration": {
                "id": id, "checkpoint_id": "c", "frame_id": format!("frame-{id}"), "name": id,
                "status": "active", "workspace_dir": "/tmp/x", "workspace_backend": "persistent",
                "scope_generation": 0, "warnings_json": "[]", "created_at": at, "updated_at": at,
            },
            "source_frame_id": source,
            "checkpoint_user_index": 0,
            "isolation_summary_json": "{\"fully_isolated\":true}",
        }))
        .unwrap()
    }

    #[test]
    fn explorations_start_only_from_the_round_source_and_never_from_one() {
        let rows = vec![row("b", "main", 2), row("a", "main", 1)];
        assert!(can_start(None, "main", false, false));
        assert!(!can_start(None, "main", true, false));
        assert!(!can_start(None, "main", false, true));
        let source = ExplorationState::for_session(&rows, "main");
        assert!(can_start(Some(&source), "main", false, false));
        let other = ExplorationState::for_session(&rows, "other");
        assert!(!can_start(Some(&other), "other", false, false));
        let inside = ExplorationState::for_session(&rows, "frame-a");
        assert!(!can_start(Some(&inside), "frame-a", false, false));
        let listed: Vec<_> = candidates(&rows, "main")
            .iter()
            .map(|row| row.exploration.id.as_str())
            .collect();
        assert_eq!(listed, ["a", "b"]);
        assert_eq!(default_name(2), "Exploration 3");
        assert_eq!(status_label("promoting"), "Being kept");
        assert_eq!(isolation_label(&rows[0]), "Fully isolated");
    }
}
