# Native conversation loop

The SwiftUI preview supports creating/opening HTTP-model and ACP conversations,
selecting an HTTP conversation's model, sending messages, seeing incremental text and tool
results, approving/denying a tool once, stopping execution, and reopening saved
history. Settings and conversations share the opt-in desktop host; the existing
WebView remains usable. The macOS composer also supports file attachments and one
queued follow-up, and the workspace has an Agent workflow approval panel.
Embedded MCP Apps, rich scientific artifact viewers and branch management remain
follow-ups. Frozen/archived conversations are read-only in the native composer.

The macOS main composer reads the saved `send_with_modifier` preference when a
conversation opens and updates when settings are saved. With it off, Enter sends;
with it on, Enter inserts a newline. Cmd/Ctrl+Enter sends in either mode;
Shift+Enter (including with Cmd/Ctrl) inserts a newline. IME marked-text
confirmation belongs to AppKit and never submits a message. Return shortcuts
apply only to the focused editor. Side chat retains its own Enter-to-send policy.

The macOS transcript renders block Markdown headings, nested/ordered/task lists,
quotes, fenced code and native tables in one selectable document. Code retains
newlines; long lines wrap within the conversation. Right-click a code block to
copy its original content or a table to copy TSV. Quote and highlight actions
work across blocks. Task state is shown as readable completion labels. Formula
rendering and inline images remain separate follow-ups; tool output stays literal.

The edit action next to the macOS conversation title opens a rename editor.
Saving uses the selected project and session IDs, trims surrounding whitespace,
and refreshes the sidebar after confirmed success. Empty names are rejected.
A failed or ambiguous response preserves the input and never retries by itself;
navigation invalidates the editor's pending callback. Immediate Escape closes
only the editor, without writing or closing its parent.

The pin action beside the title saves the explicit pin/unpin state for the
selected project and session. Pinned conversations appear once in a leading
“已置顶” section, retaining the chosen name/date order within that section.
Folder membership is preserved; unpinning restores the normal grouping.
Sidebar metadata refreshes after a confirmed rename/pin or a completed turn
without reopening the transcript, changing its history page, or clearing drafts.
Stale navigation replies are ignored. A lost pin response is not retried;
“刷新会话” reads the saved state. Older hosts and unsaved empty conversations
without a confirmed pin state leave the action disabled until a saved row is
available.

The trash action opens a confirmation listing the selected conversation. Sidebar
multi-selection also offers “删除所选会话”. The sidebar's accessibility selection
follows the checked rows in multi-selection mode and the open conversation in
normal navigation. Deletion uses the existing host checks for ownership, archives
and branches, and stops the selected conversation's running
work before removing it. A batch is sequential, not atomic: confirmed deletions
are removed locally, and the first error stops all later requests. An ambiguous
response never triggers another deletion automatically. The error banner provides
a read-only refresh; empty native drafts use an explicit scoped existence query
because their absence from saved history does not prove deletion. A still-existing
draft remains available and is checked again on later refreshes. Failed existence
reads preserve that draft. Immediate Escape cancels the confirmation without
writing. Navigation stops remaining batch requests and discards old UI callbacks;
confirmed deletion results still remove stale local draft entries in their scope.

## Transport and recovery

`wisp-dto::native_conversations` is the authoritative v1 protocol. Requests use
the authenticated `/invoke` transport established for native settings, with an
explicit project ID and the advertised `native_conversation_*` commands. The host
capabilities response advertises these separately from the settings allowlist;
raw agent commands are not exposed. Swift uses `NativeConversationClient`; WinUI
can depend on `INativeConversationClient` without referencing SwiftUI or Tauri.
Rust, Swift and C# consume fixtures under `contracts/native-conversations/v1`.

| Command | Arguments | Result |
| --- | --- | --- |
| `native_conversation_create` | optional `acp_agent_id` | New session ID; omitted uses HTTP |
| `native_conversation_rename` | `session_id`, `title` | Rename the owned, unarchived conversation |
| `native_conversation_pin` | `session_id`, boolean `pinned` | Set the owned, unarchived conversation's pin state |
| `native_conversation_delete` | `session_id` | Delete the owned conversation using existing host lifecycle checks |
| `native_conversation_exists` | `session_id` | Boolean existence; a different project's conversation is rejected |
| `native_conversation_snapshot` | `session_id`, optional `before_seq` | `Snapshot` replacement event |
| `native_conversation_send` | `session_id`, UUID `request_id`, `message` | Acceptance with host epoch and request/session IDs |
| `native_conversation_stop` | `session_id` | Successful void |
| `native_conversation_approve` | `session_id`, `approval_id`, `approved`, optional `feedback` | Successful void, once only |
| `native_conversation_acp_permission` | `session_id`, `request_id`, nullable `option_id` | Resolve the exact pending ACP option (null cancels) |
| `native_conversation_acp_answer` | `session_id`, `request_id`, `answer` | Persist one reply for the pending bridge question |
| `native_conversation_model` | `session_id`, `model_id` | Existing model list result |

The initial transport polls a bounded transcript snapshot every 350 ms while a
turn runs, 1.5 s while idle, and 2 s after a connection failure. This is incremental
rendering of persisted/coalesced text, not an SSE token-delta stream. Each snapshot
flushes the existing UI event writer, folds the same transcript as WebView, and
includes execution and approval state. It does not reload the entire outline or
change a WebView window's active project. Older pages use `next_before_seq` and
an explicit return-to-latest control. The composer is inactive while reading an
older page. Users can disable follow-latest to keep their reading position.

Snapshots **replace**, never append, the displayed latest page. Serialize reads
per session, accept increasing `sequence` values within an `epoch`, and retire an
old epoch after a host restart. A UI navigation generation rejects late responses
from the previous conversation. Recovery always reads another complete snapshot;
there is no missing delta to replay. Swift preserves unsent drafts during in-app
navigation; drafts are not persisted to disk. Persisted sent messages survive
reopening the app.

Send returns acceptance immediately; the host owns the turn through completion.
Disconnecting/closing the view does not cancel it. The host records accepted UUIDs
and message hashes for its lifetime and rejects payload changes using the same ID.
It does not automatically queue another native send while a turn is running.
Clients never automatically retry a mutation. After an ambiguous reply, keep the
draft, read `request_id` from snapshots to reconcile acceptance, and require an
explicit user decision before another send if acceptance cannot be determined.
Deduplication is not promised across host restarts. The in-memory ledger is bounded
at 1,024 sends per conversation and 512 viewed conversations; exhaustion reports
an error instead of silently evicting deduplication records.

Stop targets only the supplied session. A cancellation request that races runtime
creation is repeated until the host-owned turn completes. Approval removal checks
both project ownership and the exact one-shot approval ID under the same lock;
an old button cannot approve the next request. On macOS, “修改意见…” opens a
feedback editor and “拒绝并反馈” forwards the existing optional `feedback` field.
The editor keeps its text after a failed request and does not retry automatically.
Immediate Escape closes only the feedback editor; it does not reject the request
or close its parent. The client also checks the current session and approval ID
before submitting. This phase never grants permanent approval scopes. Normal `ask_user` questions stage an editable answer in the
composer, including the option description. Existing notes are preserved even
when the user switches options; edits made after staging are also retained.
Freeform answers use the same staging action. The card remains pending until a
later user message appears in the authoritative transcript. Answered/expired
cards are inactive; stale callbacks cannot edit another conversation's draft.
ACP interactions are an additive optional `acp` snapshot field. Native macOS can
answer a live ACP question or permission request already running in the shared
host. Permission cards preserve the agent's option IDs, labels and scope kinds;
choosing “always” explicitly sends that offered option, and cancel sends null.
The host validates session ownership and the pending request under its existing
resolver; an invalid option does not consume the request. Questions use the same
persisted bridge answer as WebView and reject empty, stale or cross-session
replies. The composer draft is unchanged. Local submission tracking prevents a
second reply to the same request, including after an ambiguous transport error;
there is no automatic retry. Navigation discards callbacks from the old view.
Old hosts without the optional field show inactive ACP questions.

The macOS model menu lists configured ACP profiles under “ACP · 新会话”. Choosing
one creates a fresh conversation and preserves the previous conversation's draft.
The host validates the exact profile before creating the session. Send and Stop
reuse the shared ACP turn pipeline, including stored binding recovery after a
host restart, profile/workspace validation and cancellation during startup.
Stop also interrupts initialization before the agent has returned a session
handle; dropping the pending launch aborts its actor and releases the child
process. A dead cached process is evicted without retaining the cache lock.
The additive `acp_agent_id` snapshot field identifies a persisted ACP binding;
a provisional choice is shown as `acp:<profile id>` until the first connection.
Follow-ups cannot be queued before that binding exists. An ACP conversation cannot
switch to an HTTP model; create a new conversation for that change. The provisional
choice is persisted separately on the frame and makes an otherwise empty draft
visible in the project sidebar after restart. It does not create a user message
or claim that ACP has connected, and it does not save unsent message text. A
successful connection atomically replaces the provisional choice with the actual
ACP binding. Both clients route the restored choice through ACP; a missing agent
profile produces an error instead of falling back to an HTTP model. Pending
choices survive project export/import and empty conversation copy/move; external
session bindings and histories are not injected into a new ACP session.

For an offline, isolated UI smoke, configure a QA-only ACP profile using Python
and `scripts/qa_native_acp.py --workspace /absolute/qa/root --log /absolute/qa/log`.
The fixture refuses other workspaces, streams a fixed answer, requests a harmless
choice for messages containing `permission`, and waits for Stop for messages
containing `wait`. It neither executes tools nor contacts a model provider.

## WinUI integration

The WinUI preview now hosts the same live loop as SwiftUI: create/open a session,
select an HTTP model, send, stop, one-shot approvals, older-page history, and
draft recovery after an ambiguous send. Reuse `NativeSettingsClient.ConnectAsync`,
then construct `NativeConversationClient` with that transport. Keep a
`ConversationCursor` per selected session, call `SnapshotAsync`, and replace items
only when `TryAccept` returns true. Maintain a separate state for historical pages.
Pass one fresh UUID for each intentional send, preserve the draft on ambiguous
transport errors, and reconcile via the snapshot's `RequestId`. Cancellation tokens
cancel the client request, **not** the agent; call `StopAsync` for that. Render
`Approvals` with their IDs and call `ApproveAsync` with the matching ID and explicit
user choice. Attachments, ACP composers and PNG share export remain follow-ups.

## Verification and manual smoke

The [2026-09-26 reliability acceptance record](design-qa/native-reliability-2026-09-26/README.md)
contains a reusable legacy 35-turn fixture, paired native/WebView screenshots,
loopback HTTP evidence for both send preferences and rapid paste, and warm
conversation-switch measurements. In-process session/window draft recovery was
verified. Draft preservation during a normal HTTP tool approval was also verified, including
navigation, feedback-editor Escape and rejection. The user manually verified Chinese IME confirmation under both send preferences;
CUA and HTTP/store checks confirmed the retained draft and one explicit send. The
candidate overlay itself was not independently recorded, so this is collaborative
manual evidence, not an automated IME test. The
performance record includes Computer Use overhead, not frame timing, and its CPU
scope excludes WebKit auxiliary processes. English preferences still leave Chinese
workspace labels; raw Usage JSON in the transcript is another recorded follow-up.

Automated tests use temporary stores and fake native transports, without API keys,
SSH hosts or external network calls. They cover ownership, duplicate-send handling,
stale approvals, ACP reply scope/expiry/duplicate handling, shared fixtures, snapshot order/host restarts, read failures,
late navigation callbacks and preservation of drafts. To render the real SwiftUI
conversation view with offline fixtures at desktop/narrow sizes and in dark mode:

```bash
WISP_NATIVE_SNAPSHOT_DIR=/tmp/wisp-native-conversation-qa \
  swift test --package-path apps/macos --filter NativeConversationRenderTests
```

Build `scripts/build_native_macos.sh`. If an older desktop host is already running,
exit it before testing the new host protocol. Open a project, create a conversation,
select an HTTP chat model, and send a small task using a configured test provider.
Confirm growing text, readable tool output and one-shot approval/denial. Stop a
long response, switch sessions/projects while another runs, and reopen the app to
check saved history. Interrupt the host connection and verify the transcript stays
visible and the draft is not automatically retransmitted. Check narrow windows,
model-menu Escape, and the ambiguous-send confirmation's immediate Escape behavior.

## Native UI alignment (2026-09-26)

The macOS workspace uses regular navigation rows and a session-actions menu for
rename, pin and delete. The composer shows a placeholder, grows with its draft
between 64 and 160 points, and retains the existing Return/IME policy. Height
measurement uses a separate text layout so it cannot alter the live editor's
wrapping or click target.

Markdown code and table blocks expose visible, accessible copy buttons alongside
the existing context-menu commands. Code containers, table padding, quote borders
and paragraph spacing now follow the WebView's reading hierarchy while retaining
one selectable AppKit document. Formula typesetting and syntax highlighting are
not included in this iteration.

The artifacts panel also projects completed Markdown tables and `$$` block
formulas from assistant messages in the displayed transcript page. These cards
are read-only and do not create database artifacts or files. Table cards open a
native table preview; formula cards explicitly show LaTeX source with a copy
action. User/tool messages, fenced and indented code, and incomplete formulas do
not create cards. Other WebView projections (CSV/FASTA/file references) remain
outside this projection's scope. Escape dismisses the preview while preserving
the panel and conversation.

The home page includes the documentation link and inline recent-session status
badges. General settings group workspace interaction and notifications, offer an
explicit send-shortcut choice, and place local environments before network
settings. See the [alignment implementation record](superpowers/plans/2026-09-26-swiftui-webview-ui-alignment-implementation.md)
for verification and remaining work.
