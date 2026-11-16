//! A run's card in the transcript, as the WebView's run card has it
//! (`ui/src/chat_render.rs`, `RunMonitorCard`): which row it sits under,
//! what it says about the run and what it offers in each state. No
//! graphics.
use std::collections::HashMap;

use wisp_dto::{RunRecord, native_conversations::Item};

use crate::i18n::{fill, tr};

/// What a run's card offers besides opening the run in the Runs tab, which
/// every card does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Ask aside about the run, with its live state.
    Ask,
    /// End the turn's wait on the run without touching it; the
    /// conversation is told when it ends.
    Background,
    /// Ask the run to stop.
    Cancel,
    /// Look through what a finished run left on its SSH host, bring files
    /// back and clean up: the Run review dialog.
    Review,
}

/// A run that is under way, as the WebView's card counts it: a turn may
/// be waiting on it.
pub fn going(status: &str) -> bool {
    matches!(status, "submitted" | "running" | "cancelling")
}

/// A run that has ended, one way or another.
pub fn ended(status: &str) -> bool {
    matches!(
        status,
        "succeeded" | "failed" | "cancelled" | "timed_out" | "lost"
    )
}

/// A call that submits a run, whose result names it.
pub fn submits(tool: &str) -> bool {
    matches!(
        tool,
        "run_in_context" | "wisp_run_in_context" | "transfer_between_contexts"
    )
}

/// A call that waits on a run already submitted.
pub fn watches(tool: &str) -> bool {
    matches!(tool, "monitor_run" | "wisp_monitor_run")
}

/// Whether the run is in the background: the host says so, or the user
/// just asked and the next snapshot has not said it yet.
pub fn backgrounded(run: &RunRecord, just_asked: bool) -> bool {
    going(&run.status) && (run.background_requested || just_asked)
}

/// What the card of `run` offers. As in the WebView: Move to background
/// while a conversation's run is going and not already moved; review of a
/// finished run on an SSH host whose workspace is still there. Cancel
/// follows the Runs tab.
pub fn actions(run: &RunRecord, backgrounded: bool) -> Vec<Action> {
    let mut offered = vec![Action::Ask];
    if going(&run.status) && !backgrounded && run.frame_id.is_some() {
        offered.push(Action::Background);
    }
    if crate::panel::can_cancel(&run.status) {
        offered.push(Action::Cancel);
    }
    if ended(&run.status) && run.kind == "ssh_direct" && crate::panel::can_clean(run) {
        offered.push(Action::Review);
    }
    offered
}

/// The icon beside a run's state, coloured by its tone.
pub fn status_icon(status: &str) -> &'static str {
    match status {
        "submitted" | "running" | "cancelling" => "loader",
        "succeeded" => "circle-check",
        "cancelled" => "circle-minus",
        "paused" => "pause",
        "failed" | "timed_out" | "lost" => "circle-alert",
        _ => "circle",
    }
}

/// Where and how long a run ran, as the WebView's card says it: context,
/// kind and time so far, and while it goes on, its last heartbeat and its
/// time limit. Times are Unix seconds.
pub fn meta(run: &RunRecord, now: i64) -> String {
    let started = run.started_at.unwrap_or(run.created_at);
    let until = run.ended_at.unwrap_or(now);
    let elapsed = crate::transfers::duration(until.saturating_sub(started).max(0) as u64);
    let mut parts = vec![
        run.context_id.clone(),
        run.kind.clone(),
        fill(tr("Elapsed {}"), &[&elapsed]),
    ];
    if going(&run.status) {
        if let Some(polled) = run.last_polled_at {
            let age = crate::transfers::duration(now.saturating_sub(polled).max(0) as u64);
            parts.push(fill(tr("Heartbeat {} ago"), &[&age]));
        }
        if let Some(limit) = run.timeout_secs.filter(|seconds| *seconds > 0) {
            let limit = crate::transfers::duration(limit as u64);
            parts.push(fill(tr("Limit {}"), &[&limit]));
        }
    }
    parts.join(" · ")
}

/// Which row draws each run's card, by row: one card per run. While a run
/// goes on, the card is under the call that waits on it (the last one),
/// or else the call that submitted it; once it has ended, it moves to the
/// call that submitted it, so an explicit `monitor_run` leaves no second
/// card. A run whose submitting call is not loaded keeps its card under
/// the call that watched it. The run's state is read from `runs`, the
/// conversation's run records, and else from the rows.
///
/// A call that submits a run and waits for it (`wait_for_completion`) names
/// its run only when it returns. While the turn runs, the conversation's
/// newest run that is going and that no row names yet is drawn under the
/// last such call still waiting, as the WebView draws a card for such a run
/// on its own, so it can be moved to the background from there.
pub fn placed(rows: &[Item], runs: &[RunRecord], turn_running: bool) -> HashMap<usize, String> {
    let mut submitted: HashMap<&str, usize> = HashMap::new();
    let mut watched: HashMap<&str, usize> = HashMap::new();
    let mut other: HashMap<&str, usize> = HashMap::new();
    let mut status: HashMap<&str, &str> = HashMap::new();
    let mut waiting = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let Some(run) = &row.run else {
            if turn_running
                && row.role == "tool"
                && row.ok.is_none()
                && row.tool_name.as_deref().is_some_and(submits)
            {
                waiting.push(index);
            }
            continue;
        };
        let id = run.id.as_str();
        status.insert(id, run.status.as_str());
        match row.tool_name.as_deref() {
            Some(tool) if submits(tool) => {
                submitted.entry(id).or_insert(index);
            }
            Some(tool) if watches(tool) => {
                watched.insert(id, index);
            }
            _ => {
                other.entry(id).or_insert(index);
            }
        }
    }
    let mut cards = HashMap::new();
    for (&id, &from_rows) in &status {
        let now = runs
            .iter()
            .find(|run| run.id == id)
            .map_or(from_rows, |run| run.status.as_str());
        let (first, then) = if ended(now) {
            (&submitted, &watched)
        } else {
            (&watched, &submitted)
        };
        if let Some(row) = first
            .get(id)
            .or_else(|| then.get(id))
            .or_else(|| other.get(id))
        {
            cards.insert(*row, id.to_owned());
        }
    }
    let mut unnamed: Vec<&RunRecord> = runs
        .iter()
        .filter(|run| going(&run.status) && !status.contains_key(run.id.as_str()))
        .collect();
    unnamed.sort_by_key(|run| run.created_at);
    for (row, run) in waiting.into_iter().rev().zip(unnamed.into_iter().rev()) {
        cards.insert(row, run.id.clone());
    }
    cards
}

/// What of a card changes its height, so the list measures its row again
/// when it changes.
pub fn shape(run: Option<&RunRecord>, backgrounded: bool) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    backgrounded.hash(&mut hasher);
    if let Some(run) = run {
        (
            &run.status,
            run.exit_code,
            run.cleaned_at.is_some(),
            run.cleanup_error.is_some(),
            run.last_poll_error.is_some(),
            run.remote_workdir.is_some(),
            run.stdout_tail.as_deref().map(str::len),
            run.stderr_tail.as_deref().map(str::len),
            &run.progress_json,
        )
            .hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(status: &str, kind: &str) -> RunRecord {
        serde_json::from_value(json!({
            "id": "r", "frame_id": "s", "context_id": "ssh:gpu", "title": "Align",
            "kind": kind, "status": status, "command": "bwa mem",
            "created_at": 100, "started_at": 100, "ended_at": null, "exit_code": null,
            "stdout_tail": null, "stderr_tail": null, "remote_workdir": "runs/r",
            "timeout_secs": null, "last_polled_at": null, "last_poll_error": null,
            "env_snapshot_json": "{}"
        }))
        .unwrap()
    }

    fn row(tool: &str, run: &str, status: &str) -> Item {
        serde_json::from_value(json!({
            "role": "tool", "text": "{}", "tool_name": tool,
            "run": {"id": run, "status": status, "owner_index": null, "needs_review": false}
        }))
        .unwrap()
    }

    fn text(role: &str) -> Item {
        serde_json::from_value(json!({"role": role, "text": "hello"})).unwrap()
    }

    #[test]
    fn a_going_run_can_be_moved_to_the_background_once() {
        for status in ["submitted", "running", "cancelling"] {
            let going = run(status, "ssh_direct");
            assert!(
                actions(&going, false).contains(&Action::Background),
                "{status}"
            );
            assert!(
                !actions(&going, true).contains(&Action::Background),
                "{status}"
            );
            assert!(
                !actions(&going, false).contains(&Action::Review),
                "{status}"
            );
        }
        // Moving and cancelling are two actions, offered side by side.
        assert_eq!(
            actions(&run("running", "local_detached"), false),
            [Action::Ask, Action::Background, Action::Cancel]
        );
        // A stopping run can still be moved, no longer cancelled again.
        assert_eq!(
            actions(&run("cancelling", "ssh_direct"), false),
            [Action::Ask, Action::Background]
        );
        // Only a conversation's run goes to the background.
        let mut loose = run("running", "ssh_direct");
        loose.frame_id = None;
        assert!(!actions(&loose, false).contains(&Action::Background));
        // A paused or draft run is not waited on.
        for status in ["paused", "draft"] {
            assert_eq!(
                actions(&run(status, "ssh_direct"), false),
                [Action::Ask, Action::Cancel]
            );
        }
    }

    #[test]
    fn a_finished_run_on_a_host_offers_its_review_and_one_here_does_not() {
        for status in ["succeeded", "failed", "cancelled", "timed_out", "lost"] {
            let remote = run(status, "ssh_direct");
            assert_eq!(
                actions(&remote, false),
                [Action::Ask, Action::Review],
                "{status}"
            );
            // Nothing to look through on this computer, after cleaning,
            // or without a workspace kept there.
            assert_eq!(
                actions(&run(status, "local_detached"), false),
                [Action::Ask]
            );
            let mut cleaned = remote.clone();
            cleaned.cleaned_at = Some(5);
            assert_eq!(actions(&cleaned, false), [Action::Ask]);
            let mut bare = remote.clone();
            bare.remote_workdir = None;
            assert_eq!(actions(&bare, false), [Action::Ask]);
            assert_eq!(actions(&run(status, "file_transfer"), false), [Action::Ask]);
        }
    }

    #[test]
    fn the_background_shows_only_while_the_run_goes_on() {
        let mut going = run("running", "ssh_direct");
        assert!(!backgrounded(&going, false));
        assert!(backgrounded(&going, true), "asked, not yet in a snapshot");
        going.background_requested = true;
        assert!(backgrounded(&going, false));
        let mut done = run("succeeded", "ssh_direct");
        done.background_requested = true;
        assert!(!backgrounded(&done, true));
    }

    #[test]
    fn each_state_has_its_own_icon_and_word() {
        let states = ["running", "succeeded", "failed", "cancelled", "paused"];
        let icons: std::collections::HashSet<_> = states.into_iter().map(status_icon).collect();
        assert_eq!(icons.len(), states.len());
        assert_eq!(status_icon("timed_out"), status_icon("lost"));
        assert_eq!(status_icon("submitted"), "loader");
        // The word for each, with a failed exit code, comes from the
        // wording the Runs tab uses.
        let (word, tone) = crate::present::run_state("failed", Some(2));
        assert_eq!(word, "failed · exit 2");
        assert_eq!(tone, crate::present::Tone::Danger);
        let (word, tone) = crate::present::run_state("succeeded", Some(0));
        assert_eq!(word, "succeeded");
        assert_eq!(tone, crate::present::Tone::Success);
    }

    #[test]
    fn the_meta_line_says_where_and_how_long() {
        let mut going = run("running", "ssh_direct");
        going.last_polled_at = Some(160);
        going.timeout_secs = Some(3600);
        assert_eq!(
            meta(&going, 165),
            "ssh:gpu · ssh_direct · Elapsed 1:05 · Heartbeat 0:05 ago · Limit 1:00:00"
        );
        // Once it has ended, its time stops and the heartbeat goes.
        going.status = "succeeded".into();
        going.ended_at = Some(130);
        assert_eq!(meta(&going, 999), "ssh:gpu · ssh_direct · Elapsed 0:30");
    }

    #[test]
    fn a_going_run_sits_under_its_wait_and_a_finished_one_under_its_submission() {
        let rows = vec![
            text("user"),
            row("run_in_context", "r", "running"),
            text("assistant"),
            row("monitor_run", "r", "running"),
            row("monitor_run", "r", "running"),
        ];
        // While it goes on: under the last wait, once.
        let going = [run("running", "ssh_direct")];
        assert_eq!(
            placed(&rows, &going, true),
            HashMap::from([(4, "r".to_owned())])
        );
        // Ended: back under its submission, no second card under the wait.
        let done = [run("succeeded", "ssh_direct")];
        assert_eq!(
            placed(&rows, &done, false),
            HashMap::from([(1, "r".to_owned())])
        );
        // The rows' state counts when the run's record is not there.
        assert_eq!(
            placed(&rows, &[], false),
            HashMap::from([(4, "r".to_owned())])
        );

        // Submitted to run on its own, with no wait: under its submission.
        let alone = vec![row("run_in_context", "a", "running")];
        assert_eq!(
            placed(&alone, &[], false),
            HashMap::from([(0, "a".to_owned())])
        );
        // Its submission not loaded: the finished card stays with its wait.
        let watched = vec![row("monitor_run", "w", "failed")];
        assert_eq!(
            placed(&watched, &[], false),
            HashMap::from([(0, "w".to_owned())])
        );

        // Two runs keep two cards.
        let both = vec![
            row("run_in_context", "a", "succeeded"),
            row("transfer_between_contexts", "b", "running"),
            row("monitor_run", "a", "succeeded"),
        ];
        assert_eq!(
            placed(&both, &[], false),
            HashMap::from([(0, "a".to_owned()), (1, "b".to_owned())])
        );
    }

    #[test]
    fn a_submission_still_waiting_shows_the_run_it_started() {
        let mut waiting: Item = serde_json::from_value(json!({
            "role": "tool", "text": "", "tool_name": "run_in_context"
        }))
        .unwrap();
        waiting.input = Some("{\"wait_for_completion\":true}".into());
        let rows = vec![
            text("user"),
            row("run_in_context", "old", "succeeded"),
            waiting,
        ];
        let mut older = run("running", "ssh_direct");
        older.id = "older".into();
        older.created_at = 50;
        let mut newest = run("running", "ssh_direct");
        newest.id = "new".into();
        let mut old = run("succeeded", "ssh_direct");
        old.id = "old".into();
        let runs = [older, newest, old];
        assert_eq!(
            placed(&rows, &runs, true),
            HashMap::from([(1, "old".to_owned()), (2, "new".to_owned())])
        );
        // Not once the turn is over: a call left without a result then
        // waits on nothing.
        assert_eq!(
            placed(&rows, &runs, false),
            HashMap::from([(1, "old".to_owned())])
        );
    }

    #[test]
    fn a_card_is_measured_again_when_what_it_shows_changes() {
        let going = run("running", "ssh_direct");
        let same = shape(Some(&going), false);
        assert_eq!(shape(Some(&going.clone()), false), same);
        assert_ne!(shape(Some(&going), true), same);
        let mut more = going.clone();
        more.stdout_tail = Some("line".into());
        assert_ne!(shape(Some(&more), false), same);
        let mut done = going.clone();
        done.status = "succeeded".into();
        assert_ne!(shape(Some(&done), false), same);
    }
}
