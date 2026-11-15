//! The side panel's tabs and the wording of what they list. No graphics.
use crate::i18n::tr;
use wisp_dto::RunRecord;
use wisp_server::desktop::ArtifactRow;

use crate::{files::size_label, home::age};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanelTab {
    #[default]
    Files,
    Runs,
    /// This computer and the SSH hosts the conversation's work may run on.
    Environments,
    Artifacts,
    Memos,
    Memory,
    Highlights,
    Notebook,
    /// Questions asked beside the conversation.
    Aside,
    /// The helpers the conversation sent off.
    Agents,
    /// The conversation's tool calls, the evidence behind its answers.
    Provenance,
}

impl PanelTab {
    pub const ALL: [PanelTab; 11] = [
        PanelTab::Files,
        PanelTab::Runs,
        PanelTab::Environments,
        PanelTab::Artifacts,
        PanelTab::Notebook,
        PanelTab::Provenance,
        PanelTab::Agents,
        PanelTab::Highlights,
        PanelTab::Memos,
        PanelTab::Memory,
        PanelTab::Aside,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PanelTab::Files => tr("Files"),
            PanelTab::Runs => tr("Runs"),
            PanelTab::Environments => tr("Environments"),
            PanelTab::Artifacts => tr("Artifacts"),
            PanelTab::Memos => tr("Memos"),
            PanelTab::Memory => tr("Memory"),
            PanelTab::Highlights => tr("Highlights"),
            PanelTab::Notebook => tr("Notebook"),
            PanelTab::Aside => tr("Aside"),
            PanelTab::Agents => tr("Agents"),
            PanelTab::Provenance => tr("Provenance"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            PanelTab::Files => "folder",
            PanelTab::Runs => "activity-orbit",
            PanelTab::Environments => "server",
            PanelTab::Artifacts => "layers",
            PanelTab::Memos => "note-plus",
            PanelTab::Memory => "memory",
            PanelTab::Highlights => "star",
            PanelTab::Notebook => "notebook",
            PanelTab::Aside => "circle-help",
            PanelTab::Agents => "swarm",
            PanelTab::Provenance => "research-trail",
        }
    }

    /// The name the tab is saved under, which wording does not change.
    pub fn key(self) -> &'static str {
        match self {
            PanelTab::Files => "files",
            PanelTab::Runs => "runs",
            PanelTab::Environments => "environments",
            PanelTab::Artifacts => "artifacts",
            PanelTab::Memos => "memos",
            PanelTab::Memory => "memory",
            PanelTab::Highlights => "highlights",
            PanelTab::Notebook => "notebook",
            PanelTab::Aside => "aside",
            PanelTab::Agents => "agents",
            PanelTab::Provenance => "provenance",
        }
    }

    pub fn from_key(key: &str) -> Option<PanelTab> {
        PanelTab::ALL.into_iter().find(|tab| tab.key() == key)
    }
}

/// The tabs the panel shows: the ones kept, in their usual order, and the
/// one open now even when it had been taken away.
pub fn visible(kept: &[PanelTab], open: PanelTab) -> Vec<PanelTab> {
    PanelTab::ALL
        .into_iter()
        .filter(|tab| kept.contains(tab) || *tab == open)
        .collect()
}

/// The tabs that can be added back.
pub fn missing(kept: &[PanelTab], open: PanelTab) -> Vec<PanelTab> {
    let shown = visible(kept, open);
    PanelTab::ALL
        .into_iter()
        .filter(|tab| !shown.contains(tab))
        .collect()
}

/// Takes `tab` away: the tabs left, and the one to show next. That is the
/// open one when another tab goes, else the tab before it, else the first;
/// `None` when no tab is left and the panel closes.
pub fn take_away(
    kept: &[PanelTab],
    open: PanelTab,
    tab: PanelTab,
) -> (Vec<PanelTab>, Option<PanelTab>) {
    let shown = visible(kept, open);
    let at = shown.iter().position(|other| *other == tab).unwrap_or(0);
    let left: Vec<PanelTab> = shown.into_iter().filter(|other| *other != tab).collect();
    let next = if left.is_empty() {
        None
    } else if open != tab {
        Some(open)
    } else {
        left.get(at.saturating_sub(1)).copied()
    };
    (left, next)
}

/// The kept tabs as saved: their keys, comma separated.
pub fn encode_tabs(tabs: &[PanelTab]) -> String {
    tabs.iter()
        .map(|tab| tab.key())
        .collect::<Vec<_>>()
        .join(",")
}

/// The kept tabs from what was saved. Nothing saved keeps every tab; a
/// name this version does not know is left out.
pub fn decode_tabs(saved: Option<&str>) -> Vec<PanelTab> {
    let Some(saved) = saved else {
        return PanelTab::ALL.to_vec();
    };
    let named: Vec<PanelTab> = saved
        .split(',')
        .filter_map(|key| PanelTab::from_key(key.trim()))
        .collect();
    PanelTab::ALL
        .into_iter()
        .filter(|tab| named.contains(tab))
        .collect()
}

/// The transcript rows the Provenance tab lists: every tool call of the
/// loaded part of the conversation, in order, as the WebView lists them.
pub fn provenance(rows: &[wisp_dto::native_conversations::Item]) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter(|(_, row)| row.role == "tool" && row.tool_name.is_some())
        .map(|(index, _)| index)
        .collect()
}

/// Whether a call's evidence shows open before it is clicked: one that
/// failed or has not finished does, as in the WebView.
pub fn provenance_open(row: &wisp_dto::native_conversations::Item) -> bool {
    row.ok != Some(true)
}

/// One kept version of an artifact, in a line: which, how large, when.
pub fn version_line(version: &wisp_server::desktop::ArtifactVersionRow, now: i64) -> String {
    let mut parts = vec![crate::i18n::fill(tr("Version {}"), &[&version.number])];
    if let Some(size) = version.size_bytes {
        parts.push(size_label(size));
    }
    parts.push(age(now, version.created_at));
    parts.join(" · ")
}

/// Whether a run can still be asked to stop. A run that is already stopping
/// or has ended cannot.
pub fn can_cancel(status: &str) -> bool {
    matches!(status, "draft" | "submitted" | "running" | "paused")
}

/// Whether a run left a workspace where it ran that can be deleted: one
/// that has ended, kept a folder there and has not been cleaned. A method
/// search and a file transfer keep none of their own.
pub fn can_clean(run: &RunRecord) -> bool {
    !matches!(run.kind.as_str(), "method_search" | "file_transfer")
        && !can_cancel(&run.status)
        && run.status != "cancelling"
        && run.cleaned_at.is_none()
        && run.remote_workdir.is_some()
}

/// Whether the host refused to clean a run only because outputs it
/// declared were never brought back, which cleaning anyway would lose.
pub fn outputs_at_stake(refusal: &str) -> bool {
    refusal.contains("never harvested")
}

/// How many of the conversation's runs have not ended, for the tab.
pub fn active_runs(runs: &[RunRecord]) -> usize {
    runs.iter()
        .filter(|run| can_cancel(&run.status) || run.status == "cancelling")
        .count()
}

/// How long to wait before reading the open conversation again. A turn is
/// followed closely; a run that outlives its turn is checked now and then
/// so its status does not go stale; and a quiet conversation is still read
/// every few seconds, because the host may start a turn in it by itself (a
/// timer, a scheduled task, a browser through remote access).
pub fn poll_delay(turn_running: bool, active_runs: usize) -> Option<std::time::Duration> {
    Some(std::time::Duration::from_millis(if turn_running {
        150
    } else if active_runs > 0 {
        1500
    } else {
        4000
    }))
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
        None => parts.push(tr("Not a project file").to_owned()),
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

    #[test]
    fn a_kept_version_says_which_how_large_and_when() {
        let version = wisp_server::desktop::ArtifactVersionRow {
            id: "v2".into(),
            number: 2,
            kind: "text/csv".into(),
            size_bytes: Some(2_048),
            created_at: 1_000,
        };
        assert_eq!(
            version_line(&version, 1_000 + 120),
            "Version 2 · 2.0 KB · 2 min ago"
        );
        let no_size = wisp_server::desktop::ArtifactVersionRow {
            size_bytes: None,
            ..version
        };
        assert_eq!(version_line(&no_size, 1_030), "Version 2 · just now");
    }

    #[test]
    fn the_provenance_tab_lists_the_tool_calls_and_opens_the_ones_in_doubt() {
        let row = |role: &str, tool: Option<&str>, ok: Option<bool>| {
            serde_json::from_value::<wisp_dto::native_conversations::Item>(serde_json::json!({
                "role": role, "text": "", "tool_name": tool, "input": null,
                "ok": ok, "status": null
            }))
            .unwrap()
        };
        let rows = [
            row("user", None, None),
            row("tool", Some("read"), Some(true)),
            row("assistant", None, None),
            row("tool", Some("shell"), Some(false)),
            row("tool", None, None),
            row("tool", Some("python"), None),
        ];
        assert_eq!(provenance(&rows), [1, 3, 5]);
        assert!(!provenance_open(&rows[1]));
        assert!(provenance_open(&rows[3]));
        assert!(provenance_open(&rows[5]));
        assert!(PanelTab::ALL.contains(&PanelTab::Provenance));
    }
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
    fn only_an_ended_run_with_a_workspace_left_can_be_cleaned() {
        let mut ended = run("failed");
        assert!(!can_clean(&ended), "no workspace was kept");
        ended.remote_workdir = Some("runs/r".into());
        assert!(can_clean(&ended));
        for status in ["running", "cancelling", "draft"] {
            let going = RunRecord {
                status: status.into(),
                ..ended.clone()
            };
            assert!(!can_clean(&going), "{status}");
        }
        for kind in ["method_search", "file_transfer"] {
            let own = RunRecord {
                kind: kind.into(),
                ..ended.clone()
            };
            assert!(!can_clean(&own), "{kind}");
        }
        ended.cleaned_at = Some(5);
        assert!(!can_clean(&ended), "already cleaned");
        assert!(outputs_at_stake(
            "Run outputs were never harvested; call harvest_run first"
        ));
        assert!(!outputs_at_stake("Run has no server workspace to clean"));
    }

    #[test]
    fn a_conversation_is_followed_while_its_turn_or_its_runs_go_on() {
        let fast = poll_delay(true, 0).unwrap();
        let slow = poll_delay(false, 2).unwrap();
        assert!(fast < slow);
        // The turn sets the pace even when runs are active too.
        assert_eq!(poll_delay(true, 3), Some(fast));
        // A quiet conversation is still read, least often.
        let quiet = poll_delay(false, 0).unwrap();
        assert!(slow < quiet);
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

    #[test]
    fn tabs_are_taken_away_and_added_back_in_their_usual_order() {
        use PanelTab::*;
        let all = PanelTab::ALL.to_vec();
        // The open tab shows even when it had been taken away.
        assert_eq!(visible(&[Runs, Files], Memos), [Files, Runs, Memos]);
        assert_eq!(missing(&all, Files), Vec::<PanelTab>::new());
        assert_eq!(missing(&[Files], Files).len(), PanelTab::ALL.len() - 1);
        assert!(!missing(&[Files], Runs).contains(&Runs));

        // Another tab goes: the open one stays.
        let (left, next) = take_away(&[Files, Runs, Artifacts], Files, Runs);
        assert_eq!((left, next), (vec![Files, Artifacts], Some(Files)));
        // The open one goes: the tab before it shows, or the first.
        let (left, next) = take_away(&[Files, Runs, Artifacts], Artifacts, Artifacts);
        assert_eq!((left, next), (vec![Files, Runs], Some(Runs)));
        let (left, next) = take_away(&[Files, Runs], Files, Files);
        assert_eq!((left, next), (vec![Runs], Some(Runs)));
        // The last one goes: the panel closes.
        assert_eq!(take_away(&[Runs], Runs, Runs), (vec![], None));
        assert_eq!(take_away(&[], Memos, Memos), (vec![], None));

        // What is kept is saved by key, and read back.
        let kept = [Files, Provenance, Aside];
        assert_eq!(encode_tabs(&kept), "files,provenance,aside");
        assert_eq!(decode_tabs(Some("aside, files,provenance,later")), kept);
        assert_eq!(decode_tabs(Some("")), Vec::<PanelTab>::new());
        assert_eq!(decode_tabs(None), all);
        for tab in PanelTab::ALL {
            assert_eq!(PanelTab::from_key(tab.key()), Some(tab));
        }
    }
}
