# Browsing conversation history

After a turn finishes, its commentary, reasoning, tool calls, and execution-plan
updates collapse into one **Processed** disclosure before the final report.
Per-round usage and context-compaction records between phases stay inside that
disclosure in their original order; they no longer create repeated summaries
with the same turn duration. Expand it to inspect the full process. The final
answer, trailing usage, approval/question cards, and dedicated run/media cards
remain separate. Active turns continue to show live progress.

While a later turn is running, earlier messages keep **Copy**, **Memory**, and
**Branch** available. Memory uses only the selected historical turn and still
requires confirmation before saving. Branch opens an independent conversation
at that checkpoint while the original turn continues in the background.
Existing restrictions for ACP sessions, exploration sessions, and branches
still apply. **Review**, **Rewind**, **Undo**, and **Start exploration** remain
unavailable during a running turn; the unfinished turn cannot become a memory
or branch checkpoint yet.

When a completed Run is already attached to its matching submission step,
`monitor_run` / `wisp_monitor_run` records and their progress messages join the
same Processed disclosure, including when reopening a conversation. Expanding
it preserves the individual tool results and their own durations; expanding
the submission also reveals the completed Run card. Active Runs and Runs
without a matching submission remain visible separately. If those cards or
other standalone content split the process, the total turn duration appears
only on the first Processed summary, never once per group.
Monitored Runs still trigger the existing results-review prompt when eligible,
even if their completed cards have just folded into the process.

Opening or reopening a conversation shows its latest messages, including when
entering through Recent conversations in another project. Switching back to a
conversation starts at the end instead of restoring an older reading position.
Within the open conversation, scrolling up still keeps your place while new
messages arrive. Use the jump-to-latest button to resume following new messages.

Running conversations also reload their latest history when opened in a new
window or revisited after switching projects. **Needs you** restores the current
native tool approval in the conversation; responding in another window removes
that card here too. Loading shows **Loading conversation…**, and a failed load
shows an inline **Retry** action instead of the new-conversation welcome screen.
History reads do not change the running Agent's message sequence. If live events
arrive during a load, the window keeps them and retries the outdated snapshot.


Long conversations load in pages and render a bounded window of turns. At the
top, **Show earlier loaded messages** reveals history already in memory;
**Load earlier messages** requests another page from the local database. History
remains available while the Agent is working. A pending request disables the
button and shows **Loading earlier messages…**. If reading or decoding a page
fails, an inline error appears beside the paging controls; click **Load earlier
messages** again to retry. Failed requests do not advance the history cursor.
Reopening a session replaces its paging request. A superseded request cannot
insert older rows, show an error, or clear the newer request's loading state,
even when both requests use the same history cursor.

## Compaction row and undo

A successful context compact leaves a timeline row. Expand it to read the
checkpoint summary, token counts, strategy, epoch, and the first kept turn.
If you have not continued the conversation, **Undo compaction** restores the
previous model context and marks the row undone. After new turns, undo is
disabled and **Rewind to before compact** uses the existing rewind confirmation
to cut the conversation at that kept turn. Escape closes only the open summary.

## Model view

After a compact, earlier bubbles stay on the full transcript but are dimmed
(`data-in-context="false"`) with a tooltip that they are represented by the
summary. **Full transcript | Model view** in the conversation header's action
group (icon tabs; in narrow panes, the top of the More menu) and the
context-usage panel switches the thread to the head epoch the model sees:
folded system prompt, the checkpoint, and the kept tail. Prune-only compaction
keeps those user and assistant turns in place and replaces old tool bodies
with collapsed **Archived tool result** rows instead of fake assistant
bubbles; it does not wrap process steps in the transcript's Processed
disclosure. Model view is
read-only — rewind, branch, edit, and explore stay on the full transcript.
New turns appear after that epoch's retained context, including live answers
and tool steps. Completing a turn refreshes the saved working set even when
the epoch number has not changed. Loading or failed model-context reads show
their own status and retry action; they never substitute the full transcript.
The epoch's system prompt and checkpoint remain at the start regardless of
the full transcript's history paging position.
The usage panel adds a line such as `Epoch n · system + checkpoint + k kept
turns` while a compaction is active. Switching conversations resets the view.

The header uses a two-position pill with history and eye icons. In conversation
panes up to 900 px wide it shows icons only; wider panes also show both labels.
Hover tips and accessible names remain available at either size, and keyboard
focus and pressed states identify the current view. The usage panel keeps its
text labels visible.

Compaction details and the usage panel refresh in the open conversation after
the epoch is saved, including automatic compaction at the end of a turn.
Undo restores the actual parent epoch, which can itself be compacted. A later
compact always receives a new epoch number, even after undo, rewind, or restart.

Manual compaction is a two-mode dialog from `/compact` or the Compact button
in the context-usage panel. **Regular compact** archives the transcript and
replaces old tool results with stubs; user and assistant turns stay in place.
**Semantic compact** always writes a `[context summary checkpoint]` plus a
short retained tail, even when prune alone would fit the window. The optional
summarization instruction appears only after you choose semantic compact.
`/compact preserve the unresolved QC blockers and exact file paths` opens on
the semantic path with that instruction filled in. Automatic compaction at 80%
still prunes first and only summarizes if the window is still full.

**Settings → Conversation** can also start a semantic compact after you switch
this conversation's model, and can prompt when you reopen a conversation that
has been idle for a configured number of hours (default 24; 0 disables the
prompt). Switching models does not compact unless that setting is on.

Once a compact starts, the dialog cannot be dismissed; after the archive and
new epoch are durable it closes and switches to Model view so the resulting
working set can be reviewed before continuing. The usage panel receives a
fresh post-compaction context estimate and breakdown rather than retaining the
pre-compaction conversation total.
The percentage measures the current context against the model's window;
the reduction on a compaction row compares before and after that compaction.
They have different denominators. Compaction immediately updates the context
estimate, including when reopening older sessions without a following usage
event. Cumulative input/output billing totals remain unchanged.
Retained-tail markers follow copied messages through earlier epochs; when a
legacy or ambiguous copy has no reliable origin, the marker remains unknown.

## Manual smoke checks

- Compact twice, undo the latest compact, and verify the parent epoch, its
  summary, and its undo action return without reopening the conversation.
- Undo and compact again; verify the new card is not marked as already undone.
- Trigger automatic compaction, let the turn finish, and expand its summary in
  the same window. Switch conversations during refresh and verify neither
  conversation receives the other's metadata or loses pending/live messages.

- Leave a native tool waiting for approval, open that running conversation in a
  new window via Needs you, and verify both its history and approval are visible.
  Respond in the original window and verify the restored card disappears.
- Switch a window to another project while a turn continues, then return and
  verify progress missed by that window has been restored.


- Open a long conversation from Recent conversations, then open one in another
  project. Verify an unvisited conversation starts at the latest message.
- Scroll up, switch conversations, and return. Verify the latest message is
  visible without clicking jump-to-latest. Scroll up again and verify new output
  does not pull you back down.
- In a conversation longer than 60 turns, repeatedly load earlier messages until
  the first question is available. Check message order and tool results, and use
  Show newer messages to return through the loaded history.
- Start a turn and load older history while it runs. Verify older messages appear
  and the live response remains intact.

Browser tests use mocked commands and synthetic history. Native macOS WebView
behavior and a user's private database still require a local smoke check.

## Moving or deleting a conversation with its artifacts

The **Move conversation** and **Delete** dialogs offer an unchecked **Also move /
Also delete associated artifacts and files** option. Without it, the existing
transcript-only behavior remains: workspace files stay on disk. Copying a
conversation remains transcript-only.

Checking the option loads a preview with artifact and file counts, paths to be
processed (including immutable version snapshots), and items that will remain in
the source project. Confirm is disabled while the preview loads or fails. Changing
the destination, dismissing the dialog, or reopening it resets this choice.

Only registered local artifacts owned by the conversation are eligible. Untracked
files are never discovered by scanning and deleted. Uploaded inputs, files changed
since their recorded checksum, missing/unverifiable paths, symlinks, external files,
and artifacts referenced by other conversations, Runs, publications, explorations,
or research graph relationships remain in the source project. A content-addressed
snapshot used by another artifact also remains there. Protected records survive
on an invisible deleted-session ownership frame so their existing references work.
There is no recursive deletion of output directories or remote/GPU/SSH data.

Moving eligible artifacts transfers their complete recorded version histories,
internal version dependencies, environment snapshots, and structured message
attachment bindings. Current workspace files retain their relative paths in the
destination; immutable snapshots receive a fresh private storage namespace.
Existing destination files or artifact identities cause the operation to stop,
without overwriting them. File staging requires rename support within the source
project volume and hard-link support within the target volume (for example NTFS,
APFS or ext4); unsupported filesystems fail safely. Historical transcript text
and tool arguments are not rewritten; arbitrary textual paths to unrelated source files are not portable.
Runs and protected artifacts remain in the source project.

The backend rechecks the preview before execution and requires exclusive activity
in the affected projects. Stop active work before moving/deleting files. A durable
journal under `.wisp/session-file-operations/` records staged file changes before
the database updates; commit receipts are written inside those same transactions.
Failed updates restore the source files; a failed source commit after a
cross-database target commit compensates the target copy. On project
open, interrupted operations restore uncommitted files or finish committed cleanup.
Recovery never overwrites newly created files; conflicts leave the originals in
the journal directory and report its location. Successful deletion is permanent;
this journal is for failure recovery, not a user-facing recycle bin. If the app
exits between the two project database commits, recovery restores the source and
keeps the already committed target copy so neither project loses data.

For multiple selected conversations, each is processed independently. If one fails,
its error is shown and only successfully deleted conversations leave the sidebar.
Shared artifacts are conservatively retained even if another selected conversation
is scheduled for deletion later.

Manual smoke: generate a figure with two versions, upload an input, and reference
another figure from a second conversation. Check both dialogs' previews; move to
an empty project and open the moved figure and its earlier version from the
transcript. Try a destination with a same-name file and verify neither copy changes.
Delete with and without the option and verify that inputs/shared/edited files remain.
Open either dialog and press Escape immediately. Open the action palette above
a confirmation and verify that one Escape closes only the palette, keeping the
confirmation open.

中文：移动或删除会话时，可勾选“一并移动／删除关联产物及文件”，先查看处理清单和
保留原因。默认不勾选。未登记文件、上传输入、共享证据、后来修改的文件、项目外路径
和符号链接均受保护；不会递归清空目录或处理远程文件。移动保留产物版本及消息附件关联，
目标同名文件不会被覆盖。删除成功后不可撤销；操作日志仅用于故障恢复。
