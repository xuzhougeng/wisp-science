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

[Automatic copies](upgrade-backups.md) now run after the main page loads. Clean
startup skips the exploration-promotion scan when no journal directories exist.
Actual file recovery remains synchronous before tools can change workspace
files; interrupted recovery and upgrades can therefore still delay startup.

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
`session-hydration.spec.ts` and `performance-diagnostics.spec.ts`. The mocks need
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
workspaces, unchanged-database backup detection, free-page reclamation, backup
size quotas, and user-visible maintenance progress need their own design and
regression coverage. Journal-less committed promotion metadata is left for a
later recovery scan; it does not require workspace file repair.
