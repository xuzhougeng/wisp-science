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

/// How a tool call came out, as a stamp says it: done, failed or
/// interrupted, running, or waiting for the user. None when the row does
/// not say (an older row with no recorded outcome is not claimed a
/// success). The duration is said apart, as the row's value.
pub fn tool_outcome(item: &Item, turn_running: bool) -> Option<(&'static str, Tone)> {
    if item.role != "tool" {
        return None;
    }
    match (item.ok, item.status.as_deref()) {
        (Some(false), _) | (None, Some("failed")) => Some((tr("Failed"), Tone::Danger)),
        (Some(true), _) | (None, Some("completed")) => Some((tr("Done"), Tone::Success)),
        (None, Some("waiting" | "pending_approval" | "needs_approval")) => {
            Some((tr("Waiting for you"), Tone::Attention))
        }
        (None, Some("running" | "pending" | "in_progress")) if turn_running => {
            Some((tr("Running"), Tone::Active))
        }
        (None, Some("running" | "pending" | "in_progress")) => {
            Some((tr("Interrupted"), Tone::Danger))
        }
        _ => None,
    }
}

/// The icon of what a tool call works on, from the shared vocabulary: a run
/// is a flask, a file a document, a command the terminal, an approval a
/// hand, a search a magnifier, another agent the swarm, anything else the
/// wrench.
pub fn tool_icon(item: &Item) -> &'static str {
    let name = item.tool_name.as_deref().unwrap_or_default();
    let kind = item.kind.as_deref().unwrap_or_default();
    if item.run.is_some()
        || matches!(
            name,
            "run_in_context"
                | "monitor_run"
                | "get_run"
                | "harvest_run"
                | "cancel_run"
                | "cleanup_run_workspace"
                | "remonitor"
                | "transfer_between_contexts"
        )
    {
        "flask"
    } else if name.contains("approv") || name.contains("permission") {
        "hand"
    } else if matches!(name, "shell" | "python" | "r" | "set_runtime_interpreter")
        || kind == "execute"
    {
        "terminal"
    } else if matches!(
        name,
        "read" | "write" | "edit" | "read_docx" | "suggest_docx_edit" | "view_image"
    ) || name.contains("file")
        || matches!(kind, "read" | "edit" | "delete" | "move")
    {
        "doc"
    } else if name.contains("search")
        || matches!(name, "grep" | "web_scan")
        || matches!(kind, "search" | "fetch")
    {
        "search"
    } else if matches!(name, "explore" | "map_items" | "task" | "Task")
        || name.contains("agent")
        || name.contains("delegate")
        || name.contains("swarm")
    {
        "swarm"
    } else {
        "wrench"
    }
}

/// The number of the entry `index` starts, when it is one of the user's
/// messages: its place among them, from 1, in the rows the view holds.
/// Every entry number comes from here, so how it is counted (and kept
/// stable over compaction, rewinding and branches) can change in one
/// place.
pub fn entry_number(rows: &[Item], index: usize) -> Option<usize> {
    if rows.get(index)?.role != "user" {
        return None;
    }
    Some(
        rows[..=index]
            .iter()
            .filter(|row| row.role == "user")
            .count(),
    )
}

/// The row of the latest entry: the last of the user's messages.
pub fn latest_entry(rows: &[Item]) -> Option<usize> {
    rows.iter().rposition(|row| row.role == "user")
}

/// The longest question set as a title, in characters (a Chinese one
/// counts as one).
pub const TITLE_QUESTION: usize = 120;

/// Whether a question is set in the serif Title role: only a short one, of
/// one paragraph and at most `TITLE_QUESTION` characters. Anything longer
/// reads as body text, so a long prompt is not a wall of serif.
pub fn title_question(text: &str) -> bool {
    let text = text.trim();
    !text.contains('\n') && text.chars().count() <= TITLE_QUESTION
}

/// What a built-in tool call works on, in the words it was given, when its
/// input names it: the command line of a shell, the path of a file read,
/// written or edited, the pattern or query of a search. None when the input
/// does not say; nothing is made up. A call from an ACP agent is already
/// named by its title.
pub fn tool_summary(item: &Item) -> Option<String> {
    if item.role != "tool" || item.kind.is_some() {
        return None;
    }
    let name = item.tool_name.as_deref()?;
    let input: serde_json::Value = serde_json::from_str(item.input.as_deref()?).ok()?;
    let keys: &[&str] = match name {
        "shell" => &["cmd", "command"],
        "read" | "write" | "edit" | "read_docx" | "suggest_docx_edit" | "view_image" => {
            &["path", "file_path"]
        }
        "grep" => &["pat", "pattern"],
        _ if name.contains("search") => &["query"],
        _ => return None,
    };
    keys.iter()
        .find_map(|key| input.get(*key)?.as_str())
        .map(one_line)
        .filter(|said| !said.is_empty())
}

/// Whether a question runs past three lines of the reading column, and is
/// folded to three until it is opened. Counted, not measured: a line of
/// body text holds about 96 Latin characters, or half as many Chinese. A
/// question set as a title is short enough never to fold.
pub fn long_question(text: &str) -> bool {
    const LINE: usize = 96;
    let lines: usize = text
        .lines()
        .map(|line| {
            let columns: usize = line
                .chars()
                .map(|ch| if (ch as u32) >= 0x1100 { 2 } else { 1 })
                .sum();
            columns.div_ceil(LINE).max(1)
        })
        .sum();
    lines > 3
}

/// How long a run has run, or ran, in milliseconds: from its start (or its
/// creation) to its end, or to `now` (Unix seconds) while it goes on. None
/// when that is no time at all, so a row can say the call's own time.
pub fn run_elapsed_ms(run: &RunRecord, now: i64) -> Option<u64> {
    let started = run.started_at.unwrap_or(run.created_at);
    let until = run.ended_at.unwrap_or(now);
    let seconds = until.saturating_sub(started);
    (seconds > 0).then(|| seconds as u64 * 1000)
}

/// The one way a duration is written as a record row's value: `1 ms`,
/// `14.0 s`, `2 min 05 s`.
pub fn duration_label(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f64 / 1000.0)
    } else {
        crate::i18n::fill(
            tr("{} min {} s"),
            &[&(ms / 60_000), &format!("{:02}", ms % 60_000 / 1000)],
        )
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
/// Whether the open conversation runs a turn the window is not following:
/// one the host started by itself, such as a subagent's report, a
/// background batch's result or a scheduled task.
pub fn unfollowed_turn(
    open: Option<&str>,
    rows: &[SessionSearchInfo],
    shown_running: bool,
) -> bool {
    !shown_running
        && open.is_some_and(|open| {
            rows.iter()
                .any(|row| row.id == open && matches!(row.status.as_str(), "running" | "needs_you"))
        })
}

pub fn pinned_first(rows: Vec<SessionSearchInfo>, pinned: &[String]) -> Vec<SessionSearchInfo> {
    let (mut first, rest): (Vec<_>, Vec<_>) =
        rows.into_iter().partition(|row| pinned.contains(&row.id));
    first.extend(rest);
    first
}

/// The fixed heights of the sidebar, in pixels at the usual body size:
/// its padding, the project's row, the search, the list's heading and the
/// Settings row, with the gaps between; and the navigation as a column of
/// seven rows of 32 px with their gaps.
const SIDEBAR_FIXED: f32 = 16. + 36. + 44. + 34. + 32. + 5. * 4.;
const SIDEBAR_NAV_ROWS: f32 = 7. * (32. + 4.);
/// The share of the sidebar the conversation list must get before the
/// navigation may stand as a column.
const SIDEBAR_LIST_SHARE: f32 = 0.6;

/// Whether the sidebar's navigation folds into one row of icons: when, as
/// a column, it would leave the conversation list less than half of the
/// sidebar's height. Reckoned from fixed heights, not from what is drawn,
/// so folding does not change the answer and nothing flickers at the
/// threshold (about 1085 px of sidebar).
pub fn nav_folds(sidebar_height: f32) -> bool {
    let list = sidebar_height - SIDEBAR_FIXED - SIDEBAR_NAV_ROWS;
    list < sidebar_height * SIDEBAR_LIST_SHARE
}

/// What marks a conversation in the sidebar where there is room only for
/// an icon: its icon, the word its tooltip says, and its tone. Running is
/// the loader, waiting for the user the waiting circle, archived and
/// pinned their own icons.
pub fn session_mark(
    row: &SessionSearchInfo,
    pinned: bool,
) -> Option<(&'static str, &'static str, Tone)> {
    let (word, tone) = session_badge(row, pinned)?;
    let icon = match row.status.as_str() {
        "running" => "loader",
        "needs_you" => "circle-ellipsis",
        "failed" => "circle-alert",
        _ if row.shelved => "inbox",
        _ => "pin",
    };
    Some((icon, word, tone))
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

/// What importing a conversation did, on the button that opens it.
pub fn import_outcome(outcome: wisp_server::desktop::ImportOutcome) -> &'static str {
    use wisp_server::desktop::ImportOutcome;
    match outcome {
        ImportOutcome::Imported => tr("Imported · open"),
        ImportOutcome::Updated => tr("Brought up to date · open"),
        ImportOutcome::Unchanged => tr("Nothing new · open"),
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

/// Whether a run's whole log can be read: a run on this computer keeps it
/// where the app can reach, and one whose workspace was cleaned up had its
/// log saved into the project first.
pub fn has_whole_log(run: &RunRecord) -> bool {
    run.kind == "local_detached" || run.cleaned_at.is_some()
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
    fn an_import_says_what_it_did() {
        use wisp_server::desktop::ImportOutcome;
        assert_eq!(
            super::import_outcome(ImportOutcome::Imported),
            "Imported · open"
        );
        assert_eq!(
            super::import_outcome(ImportOutcome::Unchanged),
            "Nothing new · open"
        );
    }

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

    #[test]
    fn a_whole_log_is_offered_for_a_local_run_and_for_one_cleaned_up() {
        let mut ran = run("succeeded", Some(0));
        ran.kind = "ssh_direct".into();
        // On the host still: only its last lines are here.
        assert!(!has_whole_log(&ran));
        // Its log was saved into the project before its workspace went.
        ran.cleaned_at = Some(9);
        assert!(has_whole_log(&ran));
        ran.kind = "local_detached".into();
        ran.cleaned_at = None;
        assert!(has_whole_log(&ran));
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
    fn entries_are_numbered_by_the_users_messages_from_one() {
        let rows = vec![
            item(json!({"role": "user", "text": "first"})),
            item(json!({"role": "assistant", "text": "a"})),
            item(json!({})),
            item(json!({"role": "user", "text": "second"})),
            item(json!({"role": "assistant", "text": "b"})),
        ];
        let numbers: Vec<_> = (0..rows.len()).map(|at| entry_number(&rows, at)).collect();
        assert_eq!(numbers, [Some(1), None, None, Some(2), None]);
        assert_eq!(entry_number(&rows, 9), None);
        assert_eq!(latest_entry(&rows), Some(3));
        assert_eq!(latest_entry(&rows[1..3]), None);
    }

    #[test]
    fn a_tool_call_takes_the_icon_of_what_it_works_on() {
        let named = |name: &str| item(json!({"tool_name": name}));
        assert_eq!(tool_icon(&named("run_in_context")), "flask");
        assert_eq!(tool_icon(&named("monitor_run")), "flask");
        assert_eq!(tool_icon(&named("write")), "doc");
        assert_eq!(tool_icon(&named("read")), "doc");
        assert_eq!(tool_icon(&named("edit")), "doc");
        assert_eq!(tool_icon(&named("shell")), "terminal");
        assert_eq!(tool_icon(&named("python")), "terminal");
        assert_eq!(tool_icon(&named("request_approval")), "hand");
        assert_eq!(tool_icon(&named("grep")), "search");
        assert_eq!(tool_icon(&named("pubmed_search_articles")), "search");
        assert_eq!(tool_icon(&named("explore")), "swarm");
        assert_eq!(tool_icon(&named("delegate_to_agents")), "swarm");
        assert_eq!(tool_icon(&named("update_plan")), "wrench");
        // An ACP agent's call is known by its kind.
        assert_eq!(
            tool_icon(&item(json!({"tool_name": "Bash", "kind": "execute"}))),
            "terminal"
        );
        assert_eq!(
            tool_icon(&item(json!({"tool_name": "Fetch", "kind": "fetch"}))),
            "search"
        );
        // Every icon is one of the shared set.
        for icon in [
            "flask", "doc", "terminal", "hand", "search", "swarm", "wrench",
        ] {
            assert!(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("assets/icons/{icon}.svg"))
                    .exists(),
                "{icon}"
            );
        }
    }

    #[test]
    fn a_tool_calls_outcome_is_a_stamp_and_unknown_is_none() {
        assert_eq!(
            tool_outcome(&item(json!({"role": "assistant"})), true),
            None
        );
        let running = item(json!({"status": "running"}));
        assert_eq!(
            tool_outcome(&running, true),
            Some(("Running", Tone::Active))
        );
        assert_eq!(
            tool_outcome(&running, false),
            Some(("Interrupted", Tone::Danger))
        );
        assert_eq!(
            tool_outcome(&item(json!({"ok": true})), false),
            Some(("Done", Tone::Success))
        );
        assert_eq!(
            tool_outcome(&item(json!({"ok": false})), false),
            Some(("Failed", Tone::Danger))
        );
        assert_eq!(
            tool_outcome(&item(json!({"status": "complete"})), false),
            None
        );
    }

    #[test]
    fn a_question_folds_past_three_lines() {
        assert!(!long_question("What does the coverage look like?"));
        assert!(!long_question("one\ntwo\nthree"));
        assert!(long_question("one\ntwo\nthree\nfour"));
        assert!(long_question(&"word ".repeat(60)));
        assert!(!long_question(&"word ".repeat(50)));
        assert!(long_question(&"字".repeat(150)));
        assert!(!long_question(&"字".repeat(120)));
        // A title is never long enough to fold.
        assert!(!long_question(&"a".repeat(TITLE_QUESTION)));
    }

    #[test]
    fn only_a_short_question_of_one_paragraph_is_a_title() {
        assert!(title_question("What does the coverage look like?"));
        assert!(title_question(&"a".repeat(TITLE_QUESTION)));
        assert!(!title_question(&"a".repeat(TITLE_QUESTION + 1)));
        // Chinese counts one a character.
        assert!(title_question(&"字".repeat(TITLE_QUESTION)));
        assert!(!title_question(&"字".repeat(TITLE_QUESTION + 1)));
        assert!(!title_question("First this.\n\nThen that."));
        assert!(!title_question("Line one\nline two"));
        // Spaces round it do not count.
        assert!(title_question("  Short.  \n"));
    }

    #[test]
    fn a_tool_calls_summary_is_what_its_input_names() {
        let call = |name: &str, input: serde_json::Value| {
            item(json!({"tool_name": name, "input": input.to_string()}))
        };
        assert_eq!(
            tool_summary(&call("shell", json!({"cmd": "bwa index  GRCh38.fa"}))).as_deref(),
            Some("bwa index GRCh38.fa")
        );
        assert_eq!(
            tool_summary(&call("read", json!({"path": "data/reads.fq"}))).as_deref(),
            Some("data/reads.fq")
        );
        assert_eq!(
            tool_summary(&call("grep", json!({"pat": "TODO"}))).as_deref(),
            Some("TODO")
        );
        assert_eq!(
            tool_summary(&call("pubmed_search_articles", json!({"query": "CRISPR"}))).as_deref(),
            Some("CRISPR")
        );
        // Nothing made up: no input, another tool, an ACP call.
        assert_eq!(tool_summary(&item(json!({"tool_name": "read"}))), None);
        assert_eq!(tool_summary(&call("end_round", json!({"note": "x"}))), None);
        assert_eq!(tool_summary(&call("shell", json!({"cmd": "  "}))), None);
        assert_eq!(
            tool_summary(&item(json!({
                "tool_name": "Run ls", "kind": "execute", "input": "{\"cmd\":\"ls\"}"
            }))),
            None
        );
    }

    #[test]
    fn the_sidebars_navigation_folds_when_the_conversations_would_get_too_little() {
        // The threshold is twice what the column and the fixed rows take.
        let threshold = (SIDEBAR_FIXED + SIDEBAR_NAV_ROWS) / (1. - SIDEBAR_LIST_SHARE);
        assert!((1050. ..1120.).contains(&threshold), "{threshold}");
        // The smallest window and the usual one fold; a tall one does not.
        assert!(nav_folds(540. - 40.));
        assert!(nav_folds(780. - 40.));
        assert!(nav_folds(threshold - 1.));
        assert!(!nav_folds(threshold));
        assert!(!nav_folds(1200.));
    }

    #[test]
    fn a_conversations_state_is_an_icon_with_its_word() {
        let row = |status: &str, shelved: bool| -> SessionSearchInfo {
            serde_json::from_value(json!({
                "id": "s", "title": "t", "status": status, "shelved": shelved,
                "project_id": "p", "project_name": "P"
            }))
            .unwrap()
        };
        assert_eq!(
            session_mark(&row("running", false), false),
            Some(("loader", "Running", Tone::Active))
        );
        assert_eq!(
            session_mark(&row("needs_you", false), false),
            Some(("circle-ellipsis", "Needs you", Tone::Attention))
        );
        assert_eq!(session_mark(&row("idle", false), false), None);
        assert_eq!(
            session_mark(&row("idle", true), false).map(|mark| mark.0),
            Some("inbox")
        );
        assert_eq!(
            session_mark(&row("idle", false), true).map(|mark| mark.0),
            Some("pin")
        );
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
    fn a_runs_time_is_none_when_it_took_none() {
        let mut record = run("succeeded", Some(0));
        record.created_at = 100;
        record.started_at = Some(100);
        record.ended_at = Some(100);
        assert_eq!(run_elapsed_ms(&record, 500), None);
        record.ended_at = Some(114);
        assert_eq!(run_elapsed_ms(&record, 500), Some(14_000));
        // While it runs, up to now.
        record.ended_at = None;
        assert_eq!(run_elapsed_ms(&record, 160), Some(60_000));
    }

    #[test]
    fn durations_pick_a_readable_unit() {
        assert_eq!(duration_label(0), "0 ms");
        assert_eq!(duration_label(999), "999 ms");
        assert_eq!(duration_label(1000), "1.0 s");
        assert_eq!(duration_label(59_999), "60.0 s");
        assert_eq!(duration_label(60_000), "1 min 00 s");
        assert_eq!(duration_label(125_000), "2 min 05 s");
        assert_eq!(duration_label(3_725_000), "62 min 05 s");
        assert_eq!(duration_label(1), "1 ms");
        assert_eq!(duration_label(14_000), "14.0 s");
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
        // A turn the host started in the open conversation is followed.
        let listed = vec![row("a", "complete", false), row("b", "running", false)];
        assert!(unfollowed_turn(Some("b"), &listed, false));
        assert!(!unfollowed_turn(Some("b"), &listed, true));
        assert!(!unfollowed_turn(Some("a"), &listed, false));
        assert!(!unfollowed_turn(None, &listed, false));
        assert!(unfollowed_turn(
            Some("q"),
            &[row("q", "needs_you", false)],
            false
        ));
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
