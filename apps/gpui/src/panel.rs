//! The side panel's tabs and the wording of what they list. No graphics.
use wisp_dto::RunRecord;
use wisp_server::desktop::ArtifactRow;

use crate::{files::size_label, home::age};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanelTab {
    #[default]
    Files,
    Runs,
    Artifacts,
    Memos,
}

impl PanelTab {
    pub const ALL: [PanelTab; 4] = [
        PanelTab::Files,
        PanelTab::Runs,
        PanelTab::Artifacts,
        PanelTab::Memos,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PanelTab::Files => "Files",
            PanelTab::Runs => "Runs",
            PanelTab::Artifacts => "Artifacts",
            PanelTab::Memos => "Memos",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            PanelTab::Files => "folder",
            PanelTab::Runs => "activity-orbit",
            PanelTab::Artifacts => "layers",
            PanelTab::Memos => "note-plus",
        }
    }
}

/// Whether a run can still be asked to stop. A run that is already stopping
/// or has ended cannot.
pub fn can_cancel(status: &str) -> bool {
    matches!(status, "draft" | "submitted" | "running" | "paused")
}

/// How many of the conversation's runs have not ended, for the tab.
pub fn active_runs(runs: &[RunRecord]) -> usize {
    runs.iter()
        .filter(|run| can_cancel(&run.status) || run.status == "cancelling")
        .count()
}

/// How long to wait before reading the conversation again, or `None` to
/// stop. A turn is followed closely; a run that outlives its turn is checked
/// now and then until it ends, so its status does not go stale.
pub fn poll_delay(turn_running: bool, active_runs: usize) -> Option<std::time::Duration> {
    if turn_running {
        Some(std::time::Duration::from_millis(150))
    } else if active_runs > 0 {
        Some(std::time::Duration::from_millis(1500))
    } else {
        None
    }
}

/// The line under an artifact's name: where it is, how big, how old, and
/// which conversation made it when the list spans the project.
pub fn artifact_detail(artifact: &ArtifactRow, now: i64, name_conversation: bool) -> String {
    let mut parts = Vec::new();
    match &artifact.path {
        // The name is already shown; the folder is what the path adds.
        Some(path) => match path.rsplit_once('/') {
            Some((folder, _)) => parts.push(format!("{folder}/")),
            None => {}
        },
        None => parts.push("Not a project file".to_owned()),
    }
    if let Some(size) = artifact.size_bytes {
        parts.push(size_label(size));
    }
    let made = age(now, artifact.created_at);
    if !made.is_empty() {
        parts.push(made);
    }
    if name_conversation && !artifact.session_title.is_empty() {
        parts.push(artifact.session_title.clone());
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(status: &str) -> RunRecord {
        serde_json::from_value(json!({
            "id": "r", "frame_id": "s", "context_id": "local", "title": "Align",
            "kind": "shell", "status": status, "command": "x",
            "created_at": 1, "started_at": null, "ended_at": null, "exit_code": null,
            "stdout_tail": null, "stderr_tail": null, "remote_workdir": null,
            "timeout_secs": null, "last_polled_at": null, "last_poll_error": null,
            "env_snapshot_json": "{}"
        }))
        .unwrap()
    }

    #[test]
    fn only_runs_that_have_not_ended_can_be_cancelled() {
        for status in ["draft", "submitted", "running", "paused"] {
            assert!(can_cancel(status), "{status}");
        }
        for status in [
            "cancelling",
            "succeeded",
            "failed",
            "cancelled",
            "timed_out",
            "lost",
        ] {
            assert!(!can_cancel(status), "{status}");
        }
        // A run that is stopping is still counted as not finished.
        let runs = [run("running"), run("cancelling"), run("succeeded")];
        assert_eq!(active_runs(&runs), 2);
        assert_eq!(active_runs(&[]), 0);
    }

    #[test]
    fn a_conversation_is_followed_while_its_turn_or_its_runs_go_on() {
        let fast = poll_delay(true, 0).unwrap();
        let slow = poll_delay(false, 2).unwrap();
        assert!(fast < slow);
        // The turn sets the pace even when runs are active too.
        assert_eq!(poll_delay(true, 3), Some(fast));
        assert_eq!(poll_delay(false, 0), None);
    }

    #[test]
    fn an_artifact_says_where_it_is_and_who_made_it() {
        let now = 1_800_000_000;
        let mut artifact = ArtifactRow {
            id: "a".into(),
            name: "volcano.png".into(),
            kind: "image/png".into(),
            path: Some("figures/de/volcano.png".into()),
            created_at: now - 600,
            size_bytes: Some(2048),
            session_id: "s".into(),
            session_title: "Differential expression".into(),
        };
        assert_eq!(
            artifact_detail(&artifact, now, false),
            "figures/de/ · 2.0 KB · 10 min ago"
        );
        assert_eq!(
            artifact_detail(&artifact, now, true),
            "figures/de/ · 2.0 KB · 10 min ago · Differential expression"
        );
        // At the project root there is no folder to name.
        artifact.path = Some("volcano.png".into());
        artifact.size_bytes = None;
        assert_eq!(artifact_detail(&artifact, now, false), "10 min ago");
        artifact.path = None;
        assert!(artifact_detail(&artifact, now, false).starts_with("Not a project file"));
        for tab in PanelTab::ALL {
            assert!(!tab.label().is_empty() && !tab.icon().is_empty());
        }
    }
}
