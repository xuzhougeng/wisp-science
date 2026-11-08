# Long conversations and background maintenance

For long autonomous turns, the conversation keeps the latest 48 activity items
eligible for live rendering and folds earlier process activity into disclosures.
Approvals, questions, final answers, and standalone Run/media cards remain visible.
Opening a large process group constructs its latest 100 items; **Show 100 earlier
activity items** reveals the next batch. This changes rendering, not saved history.
The model-context view retains its raw working-set order.

Switching conversations publishes the cached/empty view as one batch and the
persisted snapshot as another, preserving live-event revision checks. Normal
transcript row reads borrow the history, and activity boundaries and timestamps
are projected in linear passes. Rendered Markdown is cached by content, limited
to 64 entries and 2 MiB of source plus HTML per window.

The needs-you inbox coalesces refresh bursts for 150 ms and allows only one
request in flight per window, with a trailing refresh for intervening changes.
Desktop windows share a serialized backend query with a one-second cache;
changes to running/approval membership bypass it, and loading a session
invalidates it after marking the reply seen. Successful hydration immediately
clears the viewed reply from the local inbox, even if an older inbox request is
still in flight, while retaining pending approvals and questions. Other saved
metadata can take up to one second to appear. The additive `messages(frame_id, ts)` index covers
activity timestamp lookups, including marking a reply seen. Building it on an
existing large database still incurs a one-time cost on the first open.

While a turn streams, the saved-event writer no longer commits each UI event
on its own. Events it has finished merging accumulate, and every drained batch
is written as one transaction (`append_session_ui_events`), bounded to 128 rows
and to the existing 250 ms flush interval, so a slow stream still reaches the
saved transcript within one interval. Project databases keep the rollback
journal, where every autocommit insert is its own fsync, so a turn of N events
used to cost N commits; it now costs at most one commit per 250 ms tick that
finds rows waiting, plus one per 128-row batch in a burst. Rows written in one
batch share a commit timestamp. Transcript reads still flush the writer first,
the terminal Done/Error event is still written after the writer has drained,
and App context notices outside a turn keep the single-row path. A failed batch
writes nothing and leaves the sequence where it was, so later rows cannot land
past a hole.

[Automatic copies](upgrade-backups.md) now run after the main page loads. Clean
startup skips the exploration-promotion scan when no journal directories exist.
Actual file recovery remains synchronous before tools can change workspace
files; interrupted recovery and upgrades can therefore still delay startup.

## Agents pane and Run list polling

While open, the Agents pane refreshes every second. Only one read runs at a
time; refreshes requested during a slow read collapse into a single read after
it, and a reply for a conversation the pane has since left is dropped. The
subagent feed keeps a per-conversation fold of what it needs (each child's
latest instruction, answer, last activity time and tool count, and the parent's
`explore` and `consult_expert` calls with their results) and reads only the
messages appended since the previous poll. Compaction, undo, rewind and timer
turn replacement change the head-epoch log below the saved position, so that
conversation is folded again from its full history.

While a turn or transfer runs, the Run list re-reads only submitted, running and
cancelling Runs every second and merges them into the list it already shows. When
a Run starts or stops being active, the whole list is read at once; otherwise the
whole list is still read every five seconds. Each Run row stores up to 64 KiB each
of stdout and stderr tail ahead of most summary columns, so a full read still
touches every finished Run's tails; moving the tails out of the row needs its own
schema design.

The one-second Run and runtime polls (`list_runs`, `list_runtimes`) only need the
window's state scope, so they no longer load the project to get it. Loading the
project rebuilds its skill index, which reads every `SKILL.md` in the bundled,
project and global skill folders twice (once to hash it, once to parse it), and
creates the workspace and memory folders if they are missing; a Windows
retest in #1690 saw each `SKILL.md` read about 160 times in 61 seconds while a
task ran. The scope-only lookup makes the same checks as a full load: the
conversation, project and exploration still exist, the exploration has not
failed, and its workspace still resolves inside app data. It reports the same
errors. Other commands that only read the scope (Run detail, runtime
inspect/stop/dismiss, research graph and journey, memory write target, project
import) use it as well. Commands that read the project's files, skills or memory
still load the project.

## Diagnostics and regression checks

The application log includes `session transcript queried` with `sql_ms`, message
and event counts, and `session transcript loaded` with `total_ms`, item count,
encoded response bytes, and whether the request was for an older page. SQL timing
includes the store read/projection, but excludes waiting for event-writer flushes;
total timing includes those waits and response preparation.

WebView health snapshots retain lifetime `longest_task_ms` and add
`recent_longest_task_ms`, aggregated until the next minute log. They also report
the latest successful switch's paint time, projection count, retry count, loaded
item count, and latest projected row count. These are numeric diagnostics only;
no message content or session identifiers are added. Feedback automatically
attaches the current numeric performance summary alongside startup information. A switch may have a cache
publication followed by a hydrated publication, so two projections can be normal.

With permission to run tests, use the store index/backup tests, shared app query
tests, transcript projection/cache unit tests, and Playwright
`session-hydration.spec.ts` and `performance-diagnostics.spec.ts`. For polling,
use the `load_messages_since` and active Run summary store tests, the subagent
fold tests, the UI refresh guard and Run merge tests, and the slow-read case in
`subagent-activity.spec.ts`. The mocks need
no model API, SSH host, GPU, or user database. Deterministic assertions cover the
covering index, idempotent migration, lazy row counts, one in-flight inbox request,
backup timing, and interval/cumulative diagnostics.

Targeted local verification on macOS passed 41 Rust tests, all 11 tests in the
two Playwright files, and `cargo check --target wasm32-unknown-unknown` for the
UI. Browser coverage includes a 3,700-item live turn with an approval card.
These checks validate behavior and bounds; the Windows latency targets still
need measurement on the affected machine.

Manual checks on Windows and macOS:

1. Switch away from and back to a running turn with several thousand events.
   Verify the latest text and approval stay visible; expand older activity and
   verify it appears in batches. Finish the turn and check its final answer.
2. Open two project windows, view an unread reply, and trigger an approval in
   another session. Check inbox badges and confirm there is no repeated full
   message-table scan in a file-I/O trace.
3. Start with an overdue daily backup on a large project. Check the window loads
   before the daily-copy log, and that a second pass does not copy it again.
4. Exercise crash recovery with a disposable promotion journal: file rollback
   must still finish before agent work starts.

## Follow-up profiling

This implements the immediate W0–W3 work from #1690 around #1689's three reported
bottlenecks. The report's latency targets need a Windows WebView2 trace and
remeasurement on the affected data; they are not guaranteed by row-count bounds.
Item-level snapshot pagination, projected outlines, subscription/cursor changes,
and W4–W6 storage work remain separate follow-ups. WAL policy for portable/network
workspaces, unchanged-database backup detection, free-page reclamation, and
user-visible maintenance progress need their own design and regression
coverage; the daily-copy size quota and Run code snapshot retention (W6.4) are
described in [Automatic copies](upgrade-backups.md). Journal-less committed promotion metadata is left for a
later recovery scan; it does not require workspace file repair.
