# Browsing conversation history

After a turn finishes, its commentary, reasoning, tool calls, and execution-plan
updates collapse into one **Processed** disclosure before the final report.
Per-round usage and context-compaction records between phases stay inside that
disclosure in their original order; they no longer create repeated summaries
with the same turn duration. Expand it to inspect the full process. The final
answer, trailing usage, approval/question cards, and dedicated run/media cards
remain separate. Active turns continue to show live progress.

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
summary. **Full transcript | Model view** in the conversation header (and the
context-usage panel) switches the thread to the head epoch the model sees:
folded system prompt, the checkpoint, and the kept tail. Model view is
read-only — rewind, branch, edit, and explore stay on the full transcript.
The usage panel adds a line such as `Epoch n · system + checkpoint + k kept
turns` while a compaction is active. Switching conversations resets the view.

Compaction details and the usage panel refresh in the open conversation after
the epoch is saved, including automatic compaction at the end of a turn.
Undo restores the actual parent epoch, which can itself be compacted. A later
compact always receives a new epoch number, even after undo, rewind, or restart.
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
