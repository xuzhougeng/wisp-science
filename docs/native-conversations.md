# Native conversation loop

The SwiftUI preview supports creating/opening HTTP-model conversations, selecting
that conversation's model, sending messages, seeing incremental text and tool
results, approving/denying a tool once, stopping execution, and reopening saved
history. Settings and conversations share the opt-in desktop host; the existing
WebView remains usable. The macOS composer also supports file attachments and one
queued follow-up, and the workspace has an Agent workflow approval panel. ACP,
embedded MCP Apps, rich scientific artifact viewers and branch management remain
follow-ups. ACP and frozen/archived conversations are read-only in the native composer.

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

## Transport and recovery

`wisp-dto::native_conversations` is the authoritative v1 protocol. Requests use
the authenticated `/invoke` transport established for native settings, with an
explicit project ID and one of six `native_conversation_*` commands. The host
capabilities response advertises these separately from the settings allowlist;
raw agent commands are not exposed. Swift uses `NativeConversationClient`; WinUI
can depend on `INativeConversationClient` without referencing SwiftUI or Tauri.
Rust, Swift and C# consume fixtures under `contracts/native-conversations/v1`.

| Command | Arguments | Result |
| --- | --- | --- |
| `native_conversation_create` | `{}` | New session ID |
| `native_conversation_snapshot` | `session_id`, optional `before_seq` | `Snapshot` replacement event |
| `native_conversation_send` | `session_id`, UUID `request_id`, `message` | Acceptance with host epoch and request/session IDs |
| `native_conversation_stop` | `session_id` | Successful void |
| `native_conversation_approve` | `session_id`, `approval_id`, `approved`, optional `feedback` | Successful void, once only |
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
an old button cannot approve the next request. This phase never grants permanent
approval scopes. Normal `ask_user` questions display readable choices that fill
the composer; the user sends their chosen answer as the next message.

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

Automated tests use temporary stores and fake native transports, without API keys,
SSH hosts or external network calls. They cover ownership, duplicate-send handling,
stale approvals, shared fixtures, snapshot order/host restarts, read failures,
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
