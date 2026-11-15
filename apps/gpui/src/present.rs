//! Wording and tone for transcript rows, runs and conversation status, kept
//! apart from GPUI so it is covered without a graphics context.
use crate::i18n::tr;
use wisp_dto::{
    RunRecord, SessionSearchInfo,
    native_conversations::{Item, Snapshot},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Active,
    Success,
    Danger,
    Attention,
}

/// A conversation's title as one line. A title made from a first message
/// may hold line breaks; shown as they are they break every list row and
/// header the title is in.
pub fn one_line(title: &str) -> String {
    title.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn heading(item: &Item) -> String {
    item.tool_name
        .clone()
        .unwrap_or_else(|| match item.role.as_str() {
            "user" => tr("You").into(),
            "assistant" => "Wisp".into(),
            other => other.into(),
        })
}

/// Identity of a tool row's folded state. The call id survives streaming
/// updates and history pages; rows without one fall back to their position.
pub fn tool_key(item: &Item, index: usize) -> String {
    match (&item.call_id, item.seq) {
        (Some(call), _) => call.clone(),
        (None, Some(seq)) => format!("seq-{seq}-{}", heading(item)),
        (None, None) => format!("row-{index}"),
    }
}

/// Status of a tool row. A call still marked running after its turn ended was
/// interrupted, so it must not keep claiming to run.
pub fn tool_status(item: &Item, turn_running: bool) -> Option<(String, Tone)> {
    if item.role != "tool" {
        return None;
    }
    let (label, tone) = match (item.ok, item.status.as_deref()) {
        (Some(false), _) => (tr("Failed"), Tone::Danger),
        (Some(true), _) => (tr("Done"), Tone::Success),
        (None, Some("running")) if turn_running => (tr("Running"), Tone::Active),
        (None, Some("running")) => (tr("Interrupted"), Tone::Neutral),
        (None, _) => (tr("Done"), Tone::Neutral),
    };
    Some(match item.duration_ms {
        Some(ms) => (format!("{label} · {}", duration_label(ms)), tone),
        None => (label.into(), tone),
    })
}

pub fn duration_label(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f64 / 1000.0)
    } else {
        crate::i18n::fill(tr("{} min {} s"), &[&(ms / 60_000), &(ms % 60_000 / 1000)])
    }
}

/// Wraps verbatim text in a Markdown code fence longer than any backtick run
/// inside it, so tool input and output can never close the block early.
pub fn fenced(text: &str, language: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for ch in text.chars() {
        run = if ch == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat(longest.max(2) + 1);
    let end = if text.ends_with('\n') { "" } else { "\n" };
    format!("{fence}{language}\n{text}{end}{fence}")
}

/// Markdown for a transcript row. Tool rows show their input and output as
/// verbatim blocks; they are data, not prose to be interpreted.
pub fn body(item: &Item) -> String {
    if item.role != "tool" {
        return item.text.clone();
    }
    let mut parts = Vec::new();
    if let Some(input) = item.input.as_deref().filter(|input| !input.is_empty()) {
        parts.push(fenced(input, "json"));
    }
    if !item.text.is_empty() {
        parts.push(fenced(&item.text, ""));
    }
    parts.join("\n\n")
}

/// Whether two rows render identically, so a cached row height stays valid.
pub fn same_row(a: &Item, b: &Item) -> bool {
    a.role == b.role
        && a.text == b.text
        && a.tool_name == b.tool_name
        && a.input == b.input
        && a.ok == b.ok
        && a.status == b.status
        && a.duration_ms == b.duration_ms
}

/// Pinned conversations first; the host's order is kept within each group.
pub fn pinned_first(rows: Vec<SessionSearchInfo>, pinned: &[String]) -> Vec<SessionSearchInfo> {
    let (mut first, rest): (Vec<_>, Vec<_>) =
        rows.into_iter().partition(|row| pinned.contains(&row.id));
    first.extend(rest);
    first
}

/// Label beside a conversation: its activity, else why it is set apart.
pub fn session_badge(row: &SessionSearchInfo, pinned: bool) -> Option<(&'static str, Tone)> {
    session_status(&row.status).or(if row.shelved {
        Some((tr("Archived"), Tone::Neutral))
    } else if pinned {
        Some((tr("Pinned"), Tone::Neutral))
    } else {
        None
    })
}

pub fn session_status(status: &str) -> Option<(&'static str, Tone)> {
    match status {
        "running" => Some((tr("Running"), Tone::Active)),
        "needs_you" => Some((tr("Needs you"), Tone::Attention)),
        _ => None,
    }
}

pub fn run_status(run: &RunRecord) -> (String, Tone) {
    run_state(run.status.as_str(), run.exit_code)
}

/// A run's state in a word, with its exit code when that says it failed.
pub fn run_state(status: &str, exit_code: Option<i64>) -> (String, Tone) {
    let tone = match status {
        "submitted" | "running" | "cancelling" => Tone::Active,
        "succeeded" => Tone::Success,
        "failed" | "timed_out" | "lost" => Tone::Danger,
        "paused" => Tone::Attention,
        _ => Tone::Neutral,
    };
    let mut label = match status {
        "draft" => tr("draft").to_owned(),
        "submitted" => tr("submitted").to_owned(),
        "running" => tr("running").to_owned(),
        "paused" => tr("paused").to_owned(),
        "cancelling" => tr("cancelling").to_owned(),
        "succeeded" => tr("succeeded").to_owned(),
        "failed" => tr("failed").to_owned(),
        "cancelled" => tr("cancelled").to_owned(),
        "timed_out" => tr("timed out").to_owned(),
        "lost" => tr("lost").to_owned(),
        other => other.replace('_', " "),
    };
    if let Some(code) = exit_code.filter(|code| *code != 0) {
        label.push_str(&crate::i18n::fill(tr(" · exit {}"), &[&code]));
    }
    (label, tone)
}

/// Captured output for an expanded run card, as verbatim Markdown blocks.
pub fn run_output(run: &RunRecord) -> String {
    let mut parts = Vec::new();
    if let Some(command) = run.command.as_deref().filter(|text| !text.is_empty()) {
        parts.push(fenced(command, "sh"));
    }
    for (title, tail) in [("stdout", &run.stdout_tail), ("stderr", &run.stderr_tail)] {
        if let Some(tail) = tail.as_deref().filter(|text| !text.trim().is_empty()) {
            parts.push(format!("{title}\n\n{}", fenced(tail, "")));
        }
    }
    if parts.is_empty() {
        tr("No output captured yet.").into()
    } else {
        parts.join("\n\n")
    }
}

/// Whether a run's whole log can be read: only a run on this computer
/// keeps it where the app can reach.
pub fn has_whole_log(run: &RunRecord) -> bool {
    run.kind == "local_detached"
}

/// A run's whole log as it is shown: each stream that has something in it,
/// with a note when the beginning was left out.
pub fn whole_log(log: &wisp_server::desktop::RunLog) -> String {
    let mut parts = Vec::new();
    if log.cut {
        parts.push(tr("The log is long: this is its end.").to_owned());
    }
    for (title, text) in [("stdout", &log.stdout), ("stderr", &log.stderr)] {
        if !text.trim().is_empty() {
            parts.push(format!("{title}\n\n{}", fenced(text, "")));
        }
    }
    if log.stdout.trim().is_empty() && log.stderr.trim().is_empty() {
        parts.push(tr("The run wrote nothing.").to_owned());
    }
    parts.join("\n\n")
}

/// A conversation counts as read once its finished answer is on screen in the
/// active window. A pending approval still needs the user, so it stays unread.
pub fn should_mark_seen(status: &str, snapshot: Option<&Snapshot>, window_active: bool) -> bool {
    window_active
        && status == "needs_you"
        && snapshot.is_some_and(|snapshot| !snapshot.running && snapshot.approvals.is_empty())
}

#[cfg(test)]
mod tests {
    use super::one_line;

    #[test]
    fn a_title_is_shown_on_one_line() {
        assert_eq!(
            one_line("[Scheduled task: 周报]\n\nSummarize  this\tweek "),
            "[Scheduled task: 周报] Summarize this week"
        );
        assert_eq!(one_line("  "), "");
    }

    use super::*;
    use serde_json::json;

    #[test]
    fn a_whole_log_shows_each_stream_and_says_when_it_is_only_the_end() {
        let log = wisp_server::desktop::RunLog {
            stdout: "started\ndone\n".into(),
            stderr: "  ".into(),
            cut: false,
        };
        assert_eq!(whole_log(&log), "stdout\n\n```\nstarted\ndone\n```");
        let long = wisp_server::desktop::RunLog {
            stdout: "tail\n".into(),
            stderr: "warn\n".into(),
            cut: true,
        };
        assert_eq!(
            whole_log(&long),
            "The log is long: this is its end.\n\nstdout\n\n```\ntail\n```\n\nstderr\n\n```\nwarn\n```"
        );
        assert_eq!(
            whole_log(&wisp_server::desktop::RunLog::default()),
            "The run wrote nothing."
        );
    }

    fn item(value: serde_json::Value) -> Item {
        let mut base = json!({
            "role": "tool", "text": "", "tool_name": "shell",
            "input": null, "ok": null, "status": null
        });
        base.as_object_mut()
            .unwrap()
            .extend(value.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    }

    fn run(status: &str, exit_code: Option<i64>) -> RunRecord {
        serde_json::from_value(json!({
            "id": "r", "frame_id": "s", "context_id": "local", "title": "Align reads",
            "kind": "shell", "status": status, "command": "bwa mem ref.fa reads.fq",
            "created_at": 1, "started_at": null, "ended_at": null, "exit_code": exit_code,
            "stdout_tail": null, "stderr_tail": null, "remote_workdir": null,
            "timeout_secs": null, "last_polled_at": null, "last_poll_error": null,
            "env_snapshot_json": "{}"
        }))
        .unwrap()
    }

    fn snapshot(running: bool, approvals: serde_json::Value) -> Snapshot {
        serde_json::from_value(json!({
            "schema": wisp_dto::native_conversations::SCHEMA, "epoch": "e", "sequence": 1,
            "project_id": "p", "session_id": "s", "items": [], "next_before_seq": null,
            "running": running, "stopping": false, "read_only": false, "model_id": "m",
            "request_id": null, "error": null, "approvals": approvals
        }))
        .unwrap()
    }

    #[test]
    fn headings_name_the_speaker_or_tool() {
        assert_eq!(heading(&item(json!({}))), "shell");
        assert_eq!(
            heading(&item(json!({"role": "user", "tool_name": null}))),
            "You"
        );
        assert_eq!(
            heading(&item(json!({"role": "assistant", "tool_name": null}))),
            "Wisp"
        );
    }

    #[test]
    fn tool_status_reports_outcome_duration_and_interruption() {
        assert_eq!(tool_status(&item(json!({"role": "assistant"})), true), None);
        let running = item(json!({"status": "running"}));
        assert_eq!(
            tool_status(&running, true),
            Some(("Running".into(), Tone::Active))
        );
        assert_eq!(
            tool_status(&running, false),
            Some(("Interrupted".into(), Tone::Neutral))
        );
        assert_eq!(
            tool_status(
                &item(json!({"status": "complete", "ok": true, "duration_ms": 1500})),
                false
            ),
            Some(("Done · 1.5 s".into(), Tone::Success))
        );
        assert_eq!(
            tool_status(
                &item(json!({"status": "complete", "ok": false, "duration_ms": 12})),
                true
            ),
            Some(("Failed · 12 ms".into(), Tone::Danger))
        );
        // Older rows have no recorded outcome; do not claim success.
        assert_eq!(
            tool_status(&item(json!({"status": "complete"})), false),
            Some(("Done".into(), Tone::Neutral))
        );
    }

    #[test]
    fn durations_pick_a_readable_unit() {
        assert_eq!(duration_label(0), "0 ms");
        assert_eq!(duration_label(999), "999 ms");
        assert_eq!(duration_label(1000), "1.0 s");
        assert_eq!(duration_label(59_999), "60.0 s");
        assert_eq!(duration_label(60_000), "1 min 0 s");
        assert_eq!(duration_label(3_725_000), "62 min 5 s");
    }

    #[test]
    fn fences_outgrow_backticks_in_the_content() {
        assert_eq!(fenced("ls", "sh"), "```sh\nls\n```");
        assert_eq!(fenced("a\n", ""), "```\na\n```");
        assert_eq!(
            fenced("```rust\nfn x() {}\n```", ""),
            "````\n```rust\nfn x() {}\n```\n````"
        );
        assert_eq!(fenced("`````", ""), "``````\n`````\n``````");
        assert_eq!(fenced("中文 `x`", ""), "```\n中文 `x`\n```");
    }

    #[test]
    fn tool_rows_show_input_and_output_verbatim() {
        let pending = item(json!({"input": "{\"command\":\"ls\"}", "status": "running"}));
        assert_eq!(body(&pending), "```json\n{\"command\":\"ls\"}\n```");
        let done = item(json!({"input": "{}", "text": "# not a heading", "status": "complete"}));
        assert_eq!(body(&done), "```json\n{}\n```\n\n```\n# not a heading\n```");
        let prose = item(json!({"role": "assistant", "tool_name": null, "text": "**bold**"}));
        assert_eq!(body(&prose), "**bold**");
        assert_eq!(body(&item(json!({}))), "");
    }

    #[test]
    fn row_identity_tracks_everything_rendered() {
        let a = item(json!({"status": "running", "input": "{}"}));
        assert!(same_row(&a, &a.clone()));
        for change in [
            json!({"status": "complete", "input": "{}"}),
            json!({"status": "running", "input": "{}", "ok": true}),
            json!({"status": "running", "input": "{}", "duration_ms": 3}),
            json!({"status": "running", "input": "{}", "text": "out"}),
        ] {
            assert!(!same_row(&a, &item(change)));
        }
    }

    #[test]
    fn session_and_run_status_wording() {
        assert_eq!(session_status("running"), Some(("Running", Tone::Active)));
        assert_eq!(
            session_status("needs_you"),
            Some(("Needs you", Tone::Attention))
        );
        assert_eq!(session_status("complete"), None);
        assert_eq!(session_status(""), None);

        assert_eq!(
            run_status(&run("running", None)),
            ("running".into(), Tone::Active)
        );
        assert_eq!(
            run_status(&run("succeeded", Some(0))),
            ("succeeded".into(), Tone::Success)
        );
        assert_eq!(
            run_status(&run("failed", Some(2))),
            ("failed · exit 2".into(), Tone::Danger)
        );
        assert_eq!(
            run_status(&run("timed_out", None)),
            ("timed out".into(), Tone::Danger)
        );
        assert_eq!(
            run_status(&run("cancelled", None)),
            ("cancelled".into(), Tone::Neutral)
        );
    }

    #[test]
    fn pinned_conversations_lead_and_badges_prefer_activity() {
        let row = |id: &str, status: &str, shelved: bool| SessionSearchInfo {
            id: id.into(),
            project_id: "p".into(),
            project_name: "P".into(),
            title: id.into(),
            ts: 0,
            activity_at: 0,
            status: status.into(),
            shelved,
        };
        let rows = vec![
            row("a", "complete", false),
            row("b", "running", false),
            row("c", "complete", true),
            row("d", "complete", false),
        ];
        let ordered = pinned_first(rows, &["d".into(), "b".into(), "gone".into()]);
        let ids: Vec<_> = ordered.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["b", "d", "a", "c"]);
        assert_eq!(
            session_badge(&ordered[0], true),
            Some(("Running", Tone::Active))
        );
        assert_eq!(
            session_badge(&ordered[1], true),
            Some(("Pinned", Tone::Neutral))
        );
        assert_eq!(session_badge(&ordered[2], false), None);
        assert_eq!(
            session_badge(&ordered[3], true),
            Some(("Archived", Tone::Neutral))
        );
    }

    #[test]
    fn run_output_shows_command_and_nonempty_tails() {
        let mut record = run("failed", Some(1));
        assert_eq!(run_output(&record), "```sh\nbwa mem ref.fa reads.fq\n```");
        record.stdout_tail = Some("  \n".into());
        record.stderr_tail = Some("index not found".into());
        assert_eq!(
            run_output(&record),
            "```sh\nbwa mem ref.fa reads.fq\n```\n\nstderr\n\n```\nindex not found\n```"
        );
        record.command = None;
        record.stderr_tail = None;
        assert_eq!(run_output(&record), "No output captured yet.");
    }

    #[test]
    fn seen_requires_a_finished_answer_in_the_active_window() {
        let idle = snapshot(false, json!([]));
        assert!(should_mark_seen("needs_you", Some(&idle), true));
        assert!(!should_mark_seen("needs_you", Some(&idle), false));
        assert!(!should_mark_seen("complete", Some(&idle), true));
        assert!(!should_mark_seen("needs_you", None, true));
        assert!(!should_mark_seen(
            "needs_you",
            Some(&snapshot(true, json!([]))),
            true
        ));
        let approval = json!([{
            "approval_id": "a", "frame_id": "s", "message": "Run?", "tool": "shell",
            "preview": "rm -rf build"
        }]);
        assert!(!should_mark_seen(
            "needs_you",
            Some(&snapshot(false, approval)),
            true
        ));
    }

    #[test]
    fn a_tool_row_keeps_its_identity_when_rows_shift() {
        let call = item(json!({"call_id": "call-7", "seq": 4}));
        assert_eq!(tool_key(&call, 0), tool_key(&call, 9));
        let saved = item(json!({"seq": 4}));
        assert_eq!(tool_key(&saved, 0), tool_key(&saved, 9));
        assert_ne!(tool_key(&saved, 0), tool_key(&call, 0));
        // A live row with neither can only be told apart by where it is.
        let live = item(json!({}));
        assert_ne!(tool_key(&live, 0), tool_key(&live, 1));
    }
}
