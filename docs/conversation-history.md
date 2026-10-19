# Browsing conversation history

Project cards show separate **Needs you** and **Running** counts. **Needs you**
(**待处理** in Chinese) includes unread assistant replies and requests waiting for
your confirmation; it does not mean every listed conversation is still running.

The sidebar marks these conversations with an icon and a visible status label.
Choose **Needs you** above the conversation list to see all pending conversations
in the current project, including older history and sessions in collapsed groups
or branches. **All sessions** restores the normal grouping and sort order. The
topbar inbox uses the same complete list and also offers other projects. Opening
an unread reply clears its marker after the conversation loads; an outstanding
approval remains pending until you respond. Navigation, completed turns and
approval changes refresh the list, with periodic refreshes for other windows.

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

### Session health

Because the percentage drops back after every compaction, the usage panel also
shows a health line under the epoch line, for example `Health: split soon ·
compacted 3× · last freed 20%`. It counts only compactions on the current
epoch chain (undone epochs are excluded) and measures each compaction's freed
ratio as `(before − after) / before`:

- **OK**: fewer than two compactions, or compaction still frees plenty of
  room. Many compactions alone never change the level, so sessions that
  monitor long runs stay OK as long as each fold frees space.
- **Split soon**: after two or more compactions, the latest freed less than
  25% of its window, or less than half of what the previous one freed while
  also staying under 50%.
- **Split now**: the last two compactions both freed less than 15%; the
  window is above 90% again after a compaction; or a split-soon session's
  conversation already fills half the window.

Whenever the level is not OK, the panel shows the same status row as a nearly
full window, with **Compact**, **New session**, and **Handoff summary**. It is
passive: no modal and no throttling, so it stays visible until the level
recovers.

### Handoff summary

**Handoff summary** turns the model's current context (the compaction
checkpoint plus the retained turns, rendered the same way semantic compaction
feeds its summary model) into a note for a fresh session, written by the
conversation's model. The note uses four headings: goal and current stage,
completed work and confirmed decisions (with the files they touched), pending
work and next steps, and state to keep outside the conversation (facts that
belong in a project status file or memo rather than in chat).

The note opens in an editable dialog over the usage panel. An optional
instruction steers emphasis; **Regenerate** asks again with it. **Start new
session** closes the dialog and opens a new conversation whose first message is
the note as edited. Nothing is changed in the original conversation, which
stays in the sidebar. Escape closes the dialog (also while the note is still
being generated) and leaves the usage panel open; a second Escape closes the
panel. The button is unavailable while a turn is running and in ACP
conversations.

### Quote turns into a new conversation

A stretch of one conversation can seed another without copying or
summarizing it. **Quote to new chat** on a user turn starts a range and shows
a banner naming that turn; **End quote here** on the same or a later user turn
ends it (clicking an earlier turn swaps the two ends). Wisp renders the
completed turns in between as plain `User:` / `Assistant:` text, without tool
calls, tool output, images, reasoning, or compaction checkpoints, and opens a
new conversation with that text staged as a composer quote labelled with the
source conversation's title and turn range. You write the first message on
top of it; the quote is sent as part of that message. Nothing changes in the
original conversation, which stays in the sidebar and keeps its own history.

Escape or **Cancel** on the banner abandons the selection; switching to
another conversation drops it too. Very long spans are shortened in the
middle and the status line says so. The action is available on completed
turns of ordinary conversations, branches, and explorations, including while
a later turn is running; ACP conversations do not offer it.

## Manual smoke checks

- In a project with unread replies and a running conversation, verify that the
  home card shows separate labeled counts. Open the project and use **Needs you**
  to find an older unread reply, including one in a collapsed group or branch.
  Open it and check that its pending count decreases while unanswered approvals
  remain listed. Return to **All sessions** and check that grouping is preserved.
- Check the labels at a narrow sidebar width and increased UI font size in both
  Chinese and English; status text must remain visible when titles are truncated.

- Compact twice, undo the latest compact, and verify the parent epoch, its
  summary, and its undo action return without reopening the conversation.
- Undo and compact again; verify the new card is not marked as already undone.
- Compact twice where the second compact frees little room; verify the usage
  panel shows `Health: split soon` and the Compact / New session row at a low
  percentage, and that undoing the latest compact returns the line to OK.
- From that row, click **Handoff summary**: the note should name the current
  goal, finished items with their paths, the next item to resume, and state to
  write down. Edit it, click **Start new session**, and verify the new
  conversation's first user message is the edited note while the original
  conversation remains unchanged.
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

When a conversation is deleted together with its subagents and branches, the
preview covers each of them, and references between conversations deleted
together keep nothing. Each one's preview is rechecked before any is deleted
(see [Agent delegation](agent-delegation.md)). Selecting a conversation with
all of its subagents and branches, such as with **Select all**, deletes them the
same way.

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
