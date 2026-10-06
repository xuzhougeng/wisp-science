# Native conversation loop

## macOS conversation ZIP export (2026-10-06)

Use **会话操作 → 导出会话 ZIP** or `>export session zip` in Cmd+K. The preview
shows the saved conversation title, current-context message count, tool calls,
terminal events and registered artifact paths/sizes. Artifact inclusion defaults
on. Turning it off rereads the preview and excludes artifact bytes and provenance.
Unavailable files appear with their reasons in the manifest. The format matches
the existing WebView export/import: `manifest.json`, `messages.json`,
`transcript.md`, `tool-calls.json`, `terminal-events.json` and optional artifact/
provenance entries. It contains the current model context; messages in earlier
compaction epochs are not exported. Importing it does not restore execution-log
rows or pre-compaction history.

The native host binds the preview to the exact project/session, saved source
revision, artifact choice and file contents. A changed conversation or artifact
requires another preview. Export waits for an idle conversation without queued
messages, approvals or review; archived/read-only conversations can export.
The native save panel selects an absolute ZIP destination. Cancelling the panel
writes nothing and retains the preview. Escape in the panel closes only the
panel; Escape in the idle export sheet closes only that sheet. An in-flight save
holds the sheet. Navigation or closing discards late responses and retains the
conversation draft.

Both clients stage the archive beside the destination before replacing it. A
failed read/compression preserves an existing destination and removes the
temporary file. Files that grow or shrink during export are rejected instead of
silently truncating; native exports also reject same-size content changes.
Workspace-root aliases such as macOS `/var` and `/private/var` produce relative
manifest paths, keeping artifact restoration compatible with the importer.
An export cannot overwrite an included source artifact or its hardlink alias.
A confirmed save displays its exact path, byte count and SHA-256, with an explicit
**在 Finder 中显示** action. Lost/malformed/wrong-scope replies and cancellation
after dispatch retain the attempted save path and prevent another write in that
sheet, including after a fresh preview or artifact-choice change. Copy/move and
ZIP import also block replay after cancellation of a dispatched mutation.

Manual smoke: open an idle saved conversation, inspect the counts and missing
files, toggle artifact inclusion and save a ZIP. Import it into a different
synthetic project and check messages and artifact bytes. Repeat after compaction
to verify that the preview and archive contain the current context only. Change
a file after preview and check rejection; keep an existing ZIP at the destination
and verify a failed write preserves it. Cancel the save panel immediately with
Escape and verify the export sheet remains; the next Escape closes the sheet.
Check archived conversations, busy/queued/reviewing guards, unchanged drafts,
both locales/schemes and narrow windows. Packaged save-panel interaction remains
part of the final native acceptance gate.

## macOS search references (2026-10-06)

Cmd+K supports **Shift+Enter** and the result context menu's **引用到当前草稿**
for saved conversations and registered artifacts. If a command/project is
selected, Shift+Enter uses the first referenceable result, matching the WebView.
Selection rereads the current conversation's existing reference catalog and
stages only the exact advertised kind/ID, using its fresh label. Removed or hidden
results, self-session references and unavailable/read-only composers cannot stage
a reference. Source reads are bounded to a 512-byte query at a Unicode scalar
boundary. Reference selection never sends a message, navigates to another
conversation or replaces typed text; the existing composer deduplicates chips.

A failed read leaves the search open with an error. Query changes, cancellation,
closing, database/project/session navigation or loss of composer eligibility
discard late selections. A successful stage closes the search. Enter retains
normal navigation; IME candidate confirmation stays with AppKit. Search now
fits narrow windows, including its translated reference shortcut. Project file
search already exists in the Files panel, as in the WebView; this change does not
add a new file category to Cmd+K. Independent-window navigation and restoring
composer focus after staging remain follow-up work.

Manual smoke: keep unsent text in an editable conversation, search for another
conversation and an artifact, then use Shift+Enter or the context menu. Verify
the exact chip, unchanged text and absence of a send/navigation. Repeat the same
selection to check deduplication. Hide/remove a result or change conversation
while its reference read is pending; it must not stage into the new draft.
Press Enter while an IME candidate is active to confirm text only. Press Escape
immediately while reading; it closes the search and a late response has no effect.
Check both locales/schemes and narrow windows.

## macOS conversation controls and workspace search (2026-10-06)

The composer environment row opens session options. Full permission, delegation,
inline/background completion, automatic resume, automatic review and specialist
selection use `native_conversation_options` / `native_conversation_options_set`.
The full-permission switch requires an explicit confirmation explaining its
session scope and effect on waiting operations. Completion settings require
delegation; inline completion clears automatic resume. Specialists use the
advertised catalog and are locked after a conversation begins. Confirmed replies
must echo the session and requested setting; failures disable editing until a
fresh read and are never retried automatically. Read-only/history/disconnected
states cannot save, while a live conversation can change options for its tools
and delegation. Reading may be cancelled with Escape; saving keeps the sheet
open. Immediate Escape closes only the permission confirmation before options.

The same sheet has a separate **Global settings** section for memory, automatic
failure analysis and the reviewer backend. These settings affect all projects
and conversations. Failure analysis preserves its other values and bounds the
failure-rate threshold and minimum-failure count to 1–100. Memory replies must
identify the selected project and confirm the requested global preference;
memory file contents are not displayed in the composer.

Reviewer choices match the WebView: default HTTP model, follow the current
conversation, configured chat models, or a configured ACP agent. Known media
model IDs and explicit image profiles are excluded using the shared contract's
exact model rules. Switching rereads the complete reviewer persona before
saving and preserves its instructions, skills and connectors. A changed backend
selection in another window requires a fresh read; unconfirmed replies disable
further changes until explicit reconciliation. Saves are never replayed.
The reviewer picker participates in the window Escape stack and closes before
the options sheet. The options body scrolls in short/narrow windows.

Manual smoke: open conversation options, scroll to Global settings in a narrow
window, toggle memory, adjust failure thresholds and select each reviewer kind.
Reopen the sheet in a second project to check the global preference. Open the
reviewer picker and press Escape immediately; options must remain open. A second
press closes options. Verify both locales and schemes. Concurrent full-persona
writes still use the existing shared specialist command; its pre-save reread
reduces stale updates but is not an atomic compare-and-swap across windows.

Bound ACP conversations also expose their declared modes and configuration.
The native decoder preserves the protocol's `frameId`, `configOptions`,
`currentModeId` and `currentValue` keys. Select options include grouped choices;
boolean values remain booleans. Changes use the exact advertised IDs, run only
while the conversation is writable and idle, and preserve the draft. ACP agents
that expose mode through configuration have one mode selector. Unbound or older
hosts offer no unsupported settings controls; unknown configuration types are
shown without an editor.

Plan proposals render their shared entries with Markdown, status and priority.
Only the latest proposal after its owning user turn can offer decisions.
“Approve and execute” confirms leaving Plan through a fresh snapshot before
sending the current draft (or an explicit approval instruction for an empty
draft). “Save and exit” changes mode without sending. Built-in proposals use
`native_conversation_plan`; ACP proposals use an advertised non-plan mode.
Changed plans, drafts, attachments, references, read-only/running state or
navigation prevent execution. An uncertain mode or send is never replayed;
the user can inspect the mode and acknowledge an uncertain plan decision.

SwiftUI reads the shared snapshot's optional `plan_mode`, `fast_mode` and
`history_state`. Built-in conversations expose Agent/Plan and supported models
expose Fast; unavailable fields on an older host hide those controls. Running,
read-only and ACP conversations cannot change these built-in modes. Fast writes
the displayed model ID and keeps the model-default/session-override distinction.
An unconfirmed mode write blocks sending until an authoritative refresh; writes
are never replayed. The controls have a separate row to keep the send/model row
usable in narrow windows.

Historical user/assistant messages have a menu for editing into a branch,
branching after a response, and rewinding before a user message. The confirmation
captures the persisted sequence, digest and revision of the selected global turn,
including paginated history. Rewind removes conversation records, not file
changes. Existing source drafts survive, edited branch questions remain drafts,
and running latest turns cannot be branched. An unconfirmed history mutation
blocks another attempt and sending until the user explicitly checks its result.

The latest completed built-in turn also offers **Undo this turn**. Its read-only
preview lists text files to restore, created files to remove, artifact records,
unsupported files and conflicts. Conflicts disable confirmation, and the shared
host checks the file state again before applying undo. Binary and unrecorded
changes cannot be restored. The action binds the exact turn/revision, preserves
an existing draft and restores the original question only into empty input.
Older turns, ACP, running, queued and read-only conversations cannot undo.
Escape during the preview read closes only the preview; late reads are discarded.
Unconfirmed undo writes block further history changes without automatic replay.

Hosts advertising `context_view` expose **Model context** beside the composer
footer. The view reads the persisted head working set, including system and
checkpoint messages, the shared system/tool/rule/skill breakdown, and compaction
history. Built-in conversations can start regular or semantic `/compact` through
the existing turn pipeline. This sends no staged attachments/references and
retains the current draft. Semantic compaction accepts an optional retention
instruction. Completion refreshes the open context view; failures stay visible.
Compaction transcript cards show recorded before/after counts and checkpoints.

An eligible latest compaction offers undo before the conversation continues.
The confirmation binds the inspected epoch. The host compares that epoch while
holding the workflow lock, so another window's newer compaction cannot be undone
by an old confirmation. Undo preserves the transcript and draft. Archived,
frozen, running, queued, uncertain and ACP conversations cannot compact or undo;
ACP and older hosts do not offer this built-in context view. Immediate Escape
closes the compaction/undo confirmation before the context view, without writing.
Reads closed or superseded by navigation do not update a later view, and unknown
mutation results are never automatically replayed.

Native tool approvals default to **Once**. The snapshot now advertises scopes
for each exact pending approval ID. Ordinary grantable requests can select this
conversation, this project or all projects; project/global grants use the
existing persisted permission registry and can be revoked in Settings. Plans,
workflow-node decisions, image resizing and resource conflicts remain once-only.
Older hosts keep their existing once-only controls. Selecting a scope does not
write; the labelled Allow button submits the decision for the exact request.

The host validates ownership, request identity and grant eligibility before
consuming a pending request. Denial cannot create a broader grant. The Swift
client blocks another submission after an unconfirmed reply. **Recheck pending
request** performs a read only; if a fresh owned snapshot confirms that the exact
request is still pending, the user can explicitly decide again. Stale requests
and malformed scope maps are rejected. Escape closes the scope picker before
underlying surfaces, and narrow approval cards move long actions to another row.

Cmd+K now uses `native_workspace_search` to read persisted cross-project projects,
artifacts and conversations, including message-body matches beyond the five home
recents. Privacy and ranking remain in the shared host. Response query, preferred
project, ownership and duplicate identities are checked before display. Selecting
an artifact opens its exact ID in its owning conversation. Editing or closing the
search discards late results. Type `>` for native navigation/panel commands;
arrows and Enter select a result, and immediate Escape closes only the search.

Manual smoke: with an isolated fixture, find a conversation older than the home
recents by a message-body term, open an artifact in another project, use
`>terminal` and `>settings`, and press Escape immediately after opening search.
For an idle built-in conversation, save Plan/Fast, keep a draft, then open an
older message's branch/rewind confirmation and cancel with Escape. Verify the
parent and draft remain; confirm a branch and check its edited draft. Repeat in
light/dark and a narrow window. Remaining macOS scope is tracked in
[the complete workbench plan](superpowers/plans/2026-10-06-macos-workbench-parity.md).

The conversation action menu and scoped Cmd+K commands now offer **Copy to
another project** and **Move to another project**. The reviewed preview binds the
source conversation, destination project, transfer mode and saved transcript
revision, including all context epochs and persisted events. Confirmation checks
that revision under the shared workflow/agent lock. Running turns, approvals,
reviews, unconsumed queued messages and exploration conversations cannot transfer;
archived conversations may be copied but cannot be moved. Moving a mainline with
branches or an active exploration still follows the existing host restrictions.

Copy transfers saved conversation records; local files and Runs remain in the
source project. Move optionally uses the existing artifact fingerprint, collision
checks and recoverable file operation. The preview lists transferable artifacts
and files plus retained shared/uploaded/changed/unavailable items. If file preview
fails, the transcript alone may still move. File selection defaults off and
resets after rereading or changing destination. Confirmation revalidates file
state before changing it. Runs are retained in the source project.
Retained artifact or Run lineage may keep a deleted source frame internally;
conversation reads, mutations and queued sends reject that frame, including when
an old client would otherwise allocate a fresh runtime.

A move retains the current text draft for the new destination conversation
without sending it. Staged attachments and references must be cleared first
because they belong to the source project. Copy leaves the original draft in
place. A confirmed result can explicitly open the exact new frame. Closing a
confirmed move removes the source row; opening also navigates to the destination.
Unconfirmed replies block further submissions in that sheet, even after a new
preview or target selection. Closed/cancelled/superseded reads and late writes
cannot navigate another view. Saving holds the sheet open, and immediate Escape
closes the destination picker before the transfer sheet.

Manual smoke: create two temporary projects and an idle conversation with a text
draft and local generated artifacts. Copy it and inspect the destination records
while the original draft/files remain. Move another conversation with files off,
then with files on; inspect paths and retained uploads/shared artifacts. Change a
source file or transcript after preview and verify confirmation refuses stale
state. Add a destination collision and verify files remain unchanged. Check
queued/running/archived restrictions, draft retention, both locales/schemes and
immediate Escape with the destination picker open. Use only fixture projects.

The action menu and scoped Cmd+K command **Conversation relationships** show the
source conversation, sibling branches, direct branches and subagent conversations
from the saved project sidebar. Active, merged and orphaned branch states are
identified. Sidebar rows also identify branches and subagent conversations.
Opening a relationship uses its exact advertised session ID in the same project
and preserves each conversation's text draft. Missing, deleted or foreign
sources do not become navigation targets; the dialog explains an unavailable
source. Orphaned checkpoints carry an explicit explanation and do not advertise
a source or sibling link. Navigation remains available for inspecting frozen records; the host's
existing read-only controls govern any subsequent mutation.

Manual smoke: create a source, two branches and a branch from a branch. Open
**Conversation relationships** from each record and inspect the source, siblings
and direct children. Keep unsent text in two conversations and switch between
them. Inspect merged/orphaned branches and subagent records. Check both locales,
light/dark, a narrow window and immediate Escape without moving focus.


## Windows input references and preferences (2026-10-05)

The Windows input field follows the saved Enter/Ctrl+Enter preference, including
Shift+Enter and IME composition protection. The footer theme selector now reads
and updates the host's appearance preferences; entering Settings reuses the same
saved theme. Theme writes preserve typography, input options and unknown fields.
Failed writes are reported without automatic retries.

On hosts advertising `composer_references`, `@` searches artifacts and execution
environments/runtimes, `#` searches sessions and the current project, and `/`
includes enabled skills and workflows alongside the existing native commands.
Enter/Tab selects a candidate and Escape dismisses it, including while a read is
pending. References are removable chips with stable IDs, kept with each session's
draft. Sending or queueing passes the typed references to the shared desktop
resolver and persists display labels. An uncertain/cancelled send retains text
and chips until the host acknowledges the request. Old hosts keep their existing
composer behavior without exposing unsupported reference writes.

Source-aware conversation quotes, context-usage details, follow-up suggestions
and historical actions are implemented below; local-file reference parity and
the remaining real-window acceptance are tracked in
[the October 5 parity ledger](superpowers/plans/2026-10-05-winui-complete-ui-parity.md).

Focused automated coverage includes candidate identity, late responses after
query changes/navigation/Escape, UTF-16 caret positions, duplicate chip IDs,
draft restoration, typed send/queue payloads and uncertain sends. Real-window
acceptance must additionally exercise caret placement, keyboard selection,
immediate Escape, both send preferences, theme round trips and narrow layouts.

## Windows queued messages (2026-10-05)

Hosts advertising `queue` expose the shared in-memory turn queue above the
Windows composer. Up to 64 messages can wait, including messages submitted by
another window. Each row retains its own text, attachment paths and typed
references. Expand/collapse and a bounded scroll area keep the composer usable.
Older hosts keep the single-follow-up behavior.

The row menu supports editing waiting text, cancelling, moving up/down,
inserting at a safe boundary, and interrupting the current turn to prioritize
the selected message. Editing uses a dialog and preserves attachments and
references. ACP and timer turns cannot accept a cut-in. A pending cut-in is
labelled as waiting for the current step and cannot be edited or cancelled.
Escape dismisses the menu or editor immediately; navigation closes them.

Actions bind project, session, exact queue ID and payload digest. Stale edits
and already-started rows are rejected before any cancellation. Replacement
reserves priority before Stop, including when the driver already acquired its
workflow lock. Decimal-string IDs preserve all 64 bits through JSON.

Unconfirmed writes are never replayed. A snapshot can confirm a lost enqueue
response by its exact ID, without consuming a later draft. Failed dispatches
and queued payloads lost across a host restart remain available for explicit
draft recovery; recovery never sends automatically. Completed/cancelled items
are retired. Queue storage is transient; pending rows are not restored by the
host after a restart. Recovery payloads are retained only in the open native
client, not across closing that client.

Regression checks cover distinct attachment payloads with identical text,
stale actions, sorting, ACP restrictions, response loss, navigation, completion
before acknowledgement, host restart and replacement/driver exclusion. For a
manual smoke, keep an isolated ACP turn running, queue two different messages,
edit and reorder them, verify immediate Escape and disabled ACP cut-in, then
verify priority execution and completed outcomes from the host snapshot.

## Windows composer options

The WinUI 3 conversation-options popover follows the WebView row order: plan
first, full permission, delegation, completion policy, automatic review, tool
failure analysis (with conditional thresholds), reviewer model, memory,
specialist, and compute environment. Controls save through the existing desktop
commands and reread confirmed values. Failed writes show a read-only recovery
action and are never automatically retried. Navigation discards late replies.

Plan, full permission, delegation, completion, auto-review, specialist and compute
selection belong to the selected conversation. Failure-analysis settings, the
reviewer profile and memory retain their existing global scope. Full permission
requires a warning confirmation; cancelling or pressing Escape does not enable
it. Completion is disabled until delegation is on, and background completion
exposes auto-resume. Returning to inline clears auto-resume. Specialists lock
after the first message. Archived/history views cannot edit options. ACP-owned
plan mode remains under the agent's control.

Reviewer choices include the default HTTP model, following the session, named
HTTP chat models and ACP agents. Compute choices support session defaults,
remote-context membership and environment
management. Popovers are scrollable on short windows. Escape closes the child
picker/confirmation first, then the options popover, before underlying panels.

Checks: run the Windows contract-test executable and `cargo test -p wisp-dto`.
For native smoke testing, open a conversation, immediately press Escape after
opening options, and verify only the popover closes. Reopen, open a reviewer
picker and immediately press Escape: the picker should close while options stay
open. Check full-permission cancellation, disabled completion, background
auto-resume, conditional failure thresholds, saved values after reopening, and
switching conversations while a save/read is pending. Verify layout at 150%
display scaling and in a short window. Mocked transport tests do not establish
real-provider or SSH execution acceptance.

The SwiftUI preview supports creating/opening HTTP-model and ACP conversations,
selecting an HTTP conversation's model, sending messages, seeing incremental text and tool
results, approving/denying a tool once, stopping execution, and reopening saved
history. Settings and conversations share the opt-in desktop host; the existing
WebView remains usable. The macOS composer supports file attachments and multiple
queued follow-ups, and the workspace has an Agent workflow approval panel.

The composer shows the session's effective execution context and live runtime
status. Its environment menu selects among attached contexts, and the runtime
button opens the existing runtime controls without starting a process. The
reasoning menu uses the exact model ID's catalog values and updates the model
profile default for subsequent turns; models with no declared values and ACP
conversations show a disabled control. The reference button searches artifacts,
contexts/runtimes, sessions/projects, and skills/workflows. Selected references
appear as removable chips and send object IDs along with readable names. Draft
references survive session switching and unconfirmed sends; confirmed sends clear
only the submitted chips. Older hosts without the capability disable this entry.

The macOS input now opens inline candidates when a user types `@` for artifacts,
contexts/runtimes, `#` for sessions/project context, or `/` for enabled skills,
workflows and available native commands. Token boundaries and UTF-16 caret
positions follow WebView: ASCII words, email addresses, URLs and embedded paths
do not trigger a menu; Chinese text can directly precede a trigger. Pasting or
restoring a draft does not open a menu. Moving the caret away or selecting text
closes it. Search replies are isolated by query, project and conversation.

While candidates are open, Up/Down wrap through rows and reveal the selected row;
Return (including modified/keypad Return) and Tab confirm instead of sending.
Loading, error and empty states also consume these keys. Immediate window-level
Escape closes just the candidates, retains the draft and invalidates pending
searches. Clicking outside closes the menu. Chinese IME marked text owns its
keys, remains untouched by external draft updates, and resumes candidate search
after commit. Each conversation owns a separate editor so a composition cannot
commit into another conversation. Confirming a reference removes only the token
before the caret, adds its stable-ID chip and restores the caret at that position.

The slash menu groups commands, workflows and skills. Available native commands
are `/archive`, `/btw`, `/skills`, `/files`, `/upload`, `/share` and `/trajectory`,
using the same surfaces as their buttons. Actions run on selection; `/btw` fills
the input for a question and routes submission to side chat. Existing side-chat
drafts or an active side-chat request retain the new question as draft text.
These commands do not become reference chips or normal model messages. WebView
commands that need additional native APIs or confirmation surfaces (such as
`/fork`, `/rewind`, `/plan`, `/compact` and `/timer`) are not advertised by this
menu. Read-only/history/disconnected conversations and older reference hosts
retain their existing capability restrictions.

Files search filenames across the current project, including nested directories,
using the same bounded search as WebView (200 results; hidden/build folders and
symlinks are excluded). An empty query lists the current directory. Search hits
retain their project-relative paths for navigation, preview and file actions.
Artifacts remain scoped to the current conversation, with inline tables/formulas
from the displayed message page. Registered files group by directory, then inline
content by kind, and searches include display paths. Workspace files display
relative paths; remote URIs remain intact. Both collections place Save As and a
More menu beside each item, with copy-path and applicable file/provenance actions.
Save As copies original local bytes to a user-selected destination, independently
of preview truncation; remote references require retrieval before local export.

Transcript spacing is tighter around messages, user bubbles and code blocks.
Code-copy buttons reserve space on the right of wrapped code, preserving original
copy content. Composer controls use 32-point heights and panel actions use 30.

Manual regression: open a saved conversation; switch the context and inspect
runtime status; choose an available reasoning value and reopen; add references,
switch sessions, return, and verify chips. Press Escape immediately after opening
the reference picker or runtime sheet and verify only that sheet closes. Search a
nested filename and preview/rename it from results; compare artifact directory
groups and relative paths; export a file and compare bytes. Resize the window and
check multiline code, copy buttons, lists and formula selection in both themes.
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
rendering uses the shared native SwiftMath renderer for inline and block LaTeX,
preserving source for copy and selection and falling back to wrapped source for
unsupported or over-wide equations. Local Markdown images and uploaded raster
images appear inside messages, with a click-to-enlarge native preview. Completed
`generate_image` tools show their image below the tool disclosure; tool text stays
literal. Image alt text remains available for copy, quote, highlight and VoiceOver.
Loading an image preserves a text selection made before it arrived.

Images bound to message resources read the immutable artifact version captured
for that message. Missing/failed bindings never fall back to a newer file. Legacy
Markdown paths, uploaded copies and generated-tool paths read within the selected
conversation's project/exploration root. Remote URLs are not fetched. PNG, JPEG,
WebP, BMP, TIFF and the first GIF frame are supported; SVG, HEIC, animation and
full-resolution export remain follow-ups. Reads cap input at 32 MiB, bound decoder
allocation and dimensions, and return a PNG preview no larger than 1024 pixels
per side. Smaller images retain their original dimensions. Loading/unsupported/
missing images retain a readable placeholder.
Navigation discards late replies and closes the preview; Escape closes it before
an underlying overlay, without requiring focus inside.

On hosts advertising `queue`, the macOS composer displays all shared waiting
messages in order in a bounded scroll area. Enter queues while a turn runs,
following the same modifier/newline and IME policy as normal sending. Rows show
attachment names and support edit, cancel and move up/down. Editing retains
attachment paths and typed references. IDs remain decimal strings through JSON,
including values above 2^53. Actions require the exact row ID and payload digest;
started or externally edited rows cannot be changed by stale controls. Snapshots
reflect messages queued by other windows and retire started/completed rows.

Unconfirmed enqueues keep the draft and block further submission until the user
checks the queue and explicitly allows another submission. Neither enqueues nor
queue actions retry automatically. This uncertainty survives switching sessions
within the open client. The queue remains transient in the desktop host; pending
rows are not persisted across host restart. Advanced cut-in and interrupt/replace
controls remain WebView/Windows features; macOS labels pending cut-ins read-only.
Older hosts retain their single-follow-up behavior.

Smoke-test images in light/dark and narrow windows: open a message containing a
local figure, click it, immediately press Escape, select/copy text across the
figure, and reopen an older message after overwriting the original file. Attach
an image and confirm it renders after sending; remove a captured snapshot and
confirm it shows an unavailable placeholder. For queues, keep an isolated turn
running, add three follow-ups (one with files/references), edit, reorder and cancel
them, then verify execution order and session switching. Simulate lost responses
and confirm that refresh never repeats a write. These checks use offline fake
transports and AppKit rendering; they do not establish real-provider acceptance.

Fenced-code syntax highlighting loads the bundled grammar through the same
resource lookup as the native UI: installed apps use `Contents/Resources`, while
SwiftPM command-line builds use their module bundle. Opening code blocks does
not depend on the original build directory. To smoke-test a packaged build,
copy the complete app outside the build tree, make the original Swift build
directory unavailable, then open a saved conversation containing a Python code
block; verify highlighting and copy text in both themes.

Per-turn token usage appears as a compact, wrapping summary below the reply,
including input, output, reasoning, cached tokens and context window occupancy.
Usage records do not become assistant messages or expose their internal JSON.
Older records with unknown context capacity show the token count without an
invented percentage; malformed usage records are omitted. History and excerpt
navigation keep their original transcript indexes and skip usage metadata.

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
user choice. The WinUI composer also supports attachments and configured ACP
conversations. PNG share export remains a follow-up.

The ACP composer exposes session-owned permission choices and question answers.
Permission responses send the agent's exact option ID; cancellation sends null.
Answers resolve the waiting request directly. The optional snapshot `acp_state`
uses the shared camelCase `AcpSessionState` shape inside the snake_case native
snapshot, including `frameId`, `modes` and `configOptions`. The client validates
its owner before rendering. ACP session options render agent-provided mode IDs,
select options (including grouped values), and booleans; unsupported kinds are
not guessed. `native_conversation_acp_setting` validates the advertised option
and rejects active turns and read-only or archived sessions before dispatch.
Polling reconciles the result; failed/ambiguous changes are never automatically
replayed. Current mode is retained for the live agent, while a restart returns
cached available choices without pretending the old process's mode is current.

Discovered WinUI clients reread the local host descriptor before each new
request, validating its database, loopback endpoint and token together. Host
restart therefore recovers on the next snapshot poll without resending a failed
mutation. Keep ACP selector controls mounted when current values change, and
leave their popup lifecycle to WinUI during selection; the model serializes
pending writes. Escape closes the open child dropdown before its parent flyout.

Quotes retain their source session and absolute user-turn index, including older
pages, and are restored with the draft after an uncertain send. Follow-up
suggestions append to the draft without sending. Native snapshots carry a
bounded transient suggestion list which is cleared by a new user turn. The
context button summarizes active context usage rather than accumulated billing
totals. Main and auxiliary editors use the persisted send shortcut and preserve
IME composition; selecting a reference with Enter does not submit a message.

## Verification and manual smoke

WinUI message actions include branch checkpoints (before a user message or after
its reply), whole-session review, rewind-and-edit, latest-turn undo preview, and
editable project/global turn-memory proposals. These reuse the desktop commands
through `native_conversation_history_action`; no action targets an implicit
active session. Snapshot `history_state` carries durable user-row sequence IDs
and content hashes alongside absolute user indexes. The host rejects replaced
rows, cross-project requests and unsupported ACP rewind/undo. Destructive
confirmations additionally bind the conversation revision and fail if new turns
arrive while the dialog is open. Undo lists restored/removed files, artifacts,
unsupported changes and conflicts before confirmation.

Historical branch and memory actions remain usable during a later running turn.
Memory proposal generation leaves Stop available. Review, rewind and undo wait
for the session to finish. Branching does not stop the source turn; late results
cannot navigate a different selected session. Existing composer drafts survive
rewind/undo. An ambiguous mutation is never replayed, including on refresh; the
user must inspect its result and explicitly acknowledge before another history
mutation. Memory save errors retain the editor, with saving disabled while the
outcome is uncertain. Escape closes a replacement dropdown before its dialog.

History smoke: use an isolated 35-turn fixture, open both user and assistant
menus on an older page, immediately press Escape, and verify the underlying
conversation remains open. Branch from each checkpoint and inspect the source
identity and copied messages. Open a rewind/undo confirmation and cancel it with
Escape; verify the draft and transcript stay unchanged. During a later running
turn, confirm historical memory/branch remain available while review/undo are
disabled. Provider-backed review and memory generation need separate acceptance.

The 2026-09-26 reliability acceptance run
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

## WinUI execution reading (2026-10-05)

The composer shows the latest accepted `update_plan` in the current user turn.
Pending or rejected updates keep the previous accepted progress; a new actual
user turn clears it. Idle sessions retain their factual unfinished count, and
cancelled steps are distinct from completed steps. Expand the strip to read the
checklist. Only fully completed plans can be dismissed; dismissal survives
navigation within the current client model. The host projects `plan_steps` with
the same parser used by WebView, including structured nested checklists and
legacy continuation lines.

ACP tool cards retain call identity, kind, state and locations. They separate
literal text, before/after file changes, terminal IDs and resource descriptions
from an expandable original input/output view. They do not fetch referenced
resources. Failed and running tools remain readable. Multiline native text
controls enable multiline mode before assigning content, preserving logs and
diff bodies at initialization.

Snapshots include bounded output tails for the exact session-owned Runs linked
by transcript IDs. Active Runs appear inline; completed Runs move into their
exact submission disclosure, initially folded. Historical records without a
matching submission keep a standalone fallback. Cards show recorded progress,
elapsed time, command, environment and stdout/stderr; no percentages are inferred
when progress is indeterminate. Details, cancellation and result review use the
same scoped host operations as the workspace panel. Uncertain cancellation is
not replayed and does not disable the conversation's Stop control.

Older-message and outline pages keep polling the exact Runs referenced by that
page, including completed/cleaned records. These reads update only Run cards and
their matching transcript links: the selected historical messages, page cursor,
outline target and unsent draft stay in place. Duplicate links share one read,
with at most four outstanding detail requests. Page/session changes, newer
reads, cancellation and host-epoch changes discard late replies. A failed or
foreign detail reply retains the last confirmed card with a stale-state notice
and disables cancellation until a confirmed read recovers. Historical active
Runs can be cancelled through the same scoped, no-replay path as current Runs.
Standalone historical terminal cards can be dismissed even when their Run is
absent from the latest page; dismissal remains scoped to that project/session.

Selecting a question in the outline closes the outline and positions the actual
conversation at that question after layout. It no longer appends a separate
read-only transcript inside the outline. Closing the outline during its read
rejects late navigation. Older-page navigation starts at the first message;
returning to latest resumes following the conversation and restores the retained
draft's editability. Background Run reads do not repeat the positioning action.

Run-review nomination belongs to the conversation model, independent of mounted
or folded cards. A linked SSH-direct Run transitioning to success is checked
when the owning conversation is idle. The host decides whether a prompt is
needed; cleaned, read-only and stale results cannot open one. WinUI opens the
results browser directly. Immediate Escape leaves the browser for Run details;
an inner deletion confirmation consumes the first Escape and retains the parent.
Closing either a manual or automatic review persists dismissal through the host
without retries, including closing while a writable listing is still loading.
Existing dialogs and workspace overlays defer automatic opening.
Historical reading also defers automatic opening. Returning to latest can
nominate a Run observed completing on the older page; its exact record and host
review eligibility are read again even if its submission is outside the latest
transcript page. Subsequent polls do not reopen the consumed nomination.

Plan proposals from built-in `propose_plan` and ACP use the shared plan parser,
with Markdown entries, status and priority. Only the latest proposal in the
current user turn exposes decisions while plan mode is active. Approve exits
plan mode first, verifies the acknowledged mode and current proposal, then sends
the existing draft (or the default approval instruction when empty). Save and
exit preserves the draft without sending. ACP selects the advertised `default`
mode or the first non-plan mode. Running, read-only, stale, changed-draft and
uncertain outcomes cannot silently dispatch an execution turn.

Live snapshots carry an optional `activity_status` computed by the host's shared
session-status rules. The selected session's sidebar text and accessible name
update in place without reopening the session, rebuilding the root or resetting
composer focus. Older hosts can omit this field.

This remains partial reading parity: additional rich tool renderers, live
historical Run updates and packaged end-to-end acceptance remain open. Full
acceptance is tracked in the
[WinUI parity ledger](superpowers/plans/2026-10-05-winui-complete-ui-parity.md).

The WinUI file/artifact PDF preview keeps the original page bytes (up to 32 MiB)
and renders a selectable PDF.js text layer. The shared panel request's optional
`render_pdf` flag selects this behavior; omitted/false retains extracted text
for existing native clients. Select text, then use **加入聊天**
to add a removable source card without replacing or sending the draft, or
**加入聊天并跳转** to return to the composer. Cards retain the project, resolved
file path and one-based page number through navigation, sending and queued turns.
The native panel supplies the file identity; a document cannot choose a different
project/session or path through its renderer message. Closing/replacing a preview
rejects its late callbacks. Quotes are limited to 32,768 characters each, 16 cards
and 65,536 characters in total. Escape clears an active selection before closing
the preview; the panel stays open. Image-only pages remain readable and explicitly
report that there is no selectable text. This does not add OCR.

WinUI file and artifact previews also render DOCX, XLSX and PPTX offline using
the same Office rendering module as the WebView. Word preserves page layout,
tables, embedded images and supported Word equations; the renderer's existing
partial support for WPS equations still applies. Excel provides sheet tabs,
merged cells, a virtualized grid and the selected cell's existing formula. It
shows cached values without calculating formulas or fetching linked workbooks.
Large worksheets display a bounded-preview notice. PowerPoint uses lazy slide
rendering and a width-constrained scroll surface.
Opening a document brings its preview into view below the file list; ordinary
refreshes preserve the reader's position.

The optional `render_office` panel flag requests validated OOXML bytes with a
32 MiB input limit. It defaults to false independently of `render_pdf`, retaining
document extraction for older clients. The native sandbox ships the Office
bundles, shared ZIP module, worker and licenses; it cannot navigate away, open
new windows or load external document resources. Switching/closing a preview
terminates workbook parsing and disposes slide resources, and late results cannot
replace the new preview. Escape returns to the containing panel. Office previews
are read-only; the PDF-specific add-to-chat toolbar does not appear for Office.

WinUI and WebView share offline scientific renderers for SMILES/MOL/SDF molecule
drawings, PDB/mmCIF/MOL2 structures, FASTA text and aligned FASTA/CLUSTAL/Stockholm
sequences. The structure viewer supports drag rotation and wheel zoom. Alignment
previews show named, colored residues with 50-position navigation; unequal-length
rows, duplicate FASTA names and incomplete input produce explicit errors rather
than an apparently complete scientific result. The shared Nightingale integration
supplies sequence records after component initialization, so the actual alignment
canvas is populated in both clients.

The native preview places **查看源文本** and **关闭预览** above the scientific
viewer. Source mode retains the existing text editor and **返回交互预览** returns
to the visualization. This does not save the file or send a chat message.
Scientific input is bounded to the existing 1 MiB text read, with additional
limits of 512 aligned sequences, 20,000 positions, 50,000 structure atoms and
10,000 FASTA lines. Multi-record SDF requires opening one molecule at a time.

RDKit runs in a disposable worker with a 15-second parsing deadline. Structures
and alignments use fixed bundled iframe documents; closing/replacing the preview
removes the frame, message listeners and parsing resources. The native WebView2
resource policy blocks external origins, including worker requests. Escape from
a focused scientific iframe closes the preview while leaving Files open. These
fixed viewers do not provide MCP Apps integration. Scientific source quoting,
Office source quoting and the broader workbench acceptance gates remain separate.

## Windows workspace search and command palette

`Ctrl+K` searches visible projects, saved artifacts, session titles and message
bodies through `wisp.native-search.v1`. Session results use the shared store's
current-project preference, title-before-body ranking and activity ordering,
including projects with their own databases. Hidden projects are excluded before
the result limit and privacy is checked again before returning results. Failed
reads show an error; a changed query or closed overlay cannot receive old rows.

Click or Enter opens the exact owner project/session; an artifact opens its
original preview. `Shift+Enter` attaches a session/artifact reference to the
current editable draft without navigating or sending. `Ctrl+Enter` opens a
project/session in a separate native window using the same database, leaving
the original window and draft intact. Escape closes only the search overlay.

`Ctrl+Shift+P` or **命令面板** opens searchable native commands. The 25 currently
implemented routes include independent windows, settings, project navigation,
library/calendar, workspace panels, terminal, theme selection and feedback
drafting. Commands that need a project/session are absent when unavailable.
English keywords also match Chinese labels. Mouse selection and Enter from the
result list both execute the selected action. The build-time native asset check
also verifies icons named by the command registry, preventing missing resources
from breaking the palette.

Export/setup/privacy/update/font commands and remaining WebView
actions still need implementation. This search batch does not complete Research
Assistant, publication editing, MCP Apps or final packaged/DPI acceptance.

### Windows and macOS session ZIP import

On Windows use **导入会话归档…** in the project menu or command palette. On macOS
use **导入会话 ZIP 归档** in the project sidebar or `>import session archive` in
the command palette. Choose a Wisp session-export ZIP and an explicit
destination project. Preview shows the
message/artifact counts, first four user/assistant messages (600 characters each),
artifact paths and whether the selected project already contains the import.
Changing the path or destination clears the preview. Preview itself writes no
conversation data; explicit confirmation imports the reviewed file. The result
offers **打开导入的会话** and lists artifacts that could not be restored.

`wisp.native-session-import.v1` carries the destination, source session ID and
SHA-256 from preview. A changed file is rejected before any import, and artifacts
are extracted from the same verified bytes. Repeat imports are scoped to the
destination project; legacy mappings are used only if their frame belongs there.
Existing imports update only when the archive contains more messages. Existing
files are preserved, with collisions routed to `imports/<source-session>/` when
available. Archives are bounded to 256 MiB compressed, 512 MiB expanded and 4096
entries; manifest/message text is bounded to 64 MiB each.

Frozen project mainlines and archived, branch, exploration, ACP-bound, running,
reviewing or queued target conversations cannot be updated. An uncertain response
retains the selected path and destination and disables further writes in that
dialog; a fresh preview can locate an existing imported session for inspection.
Escape closes the destination dropdown before the sheet; closing the sheet
retains the original conversation draft. This route handles Wisp ZIP archives.
macOS also discards cancelled/closed/superseded previews and prevents a late
import result from navigating another conversation. Changing file/destination
or rereading after an uncertain write cannot enable another import in that sheet.

macOS smoke: import a synthetic session ZIP into two different projects, verify
the counts/first-message previews, reopen an existing import and update it with
a longer archive. Check the result's missing-artifact list and open its exact
destination. Keep a draft in the originating conversation and verify it remains
when returning. Open the destination picker and press Escape immediately; only
the picker closes. A second Escape closes the import sheet. Check narrow windows,
both locales/schemes, frozen destinations and an archive changed after preview.

### Windows and macOS Codex and Claude session import

On Windows use **导入 Codex / Claude 会话…** in the project menu or command
palette. On macOS use **导入 Codex / Claude 会话** in the project sidebar or
`>import codex` / `>import claude` in the command palette. Choose
the destination project, Codex CLI or Claude Code, and a local or registered
WSL/SSH source. The initial list uses the metadata cache; **重新扫描来源** scans
up to 500 recent source files. Filtering matches title, working directory,
session ID and path, with 25 results per page. Source reads remain bounded to
32 MiB per conversation; local paths must stay within the provider's session
root and cannot select Claude subagent logs.

**预览会话** reads the source without writing conversation data and shows the
first four user/assistant messages, up to 600 characters each. A single import
binds the selected project/provider/environment/path, source ID and SHA-256;
source changes after preview are rejected. Changing a selector, filter or page
clears the old preview. Existing-import status and updates are scoped to the
destination project, including legacy mappings shared with the WebView client.

**导入筛选结果中的待导入会话** processes all filtered new/updatable rows across
pages, obtaining a fresh preview before each write. Progress separates created,
updated, skipped and failed results. **停止后续导入** finishes the current write
and starts no further one. Reads may fail independently; any unconfirmed write
stops the batch and disables further writes in that sheet, even after a refresh.
It is never replayed. Confirmed results can open their exact destination session.
Busy/read-only target protections match ZIP imports. Escape closes an open
selector before the sheet; closing the sheet retains the originating draft and
prevents late replies from navigating the window.

macOS smoke: with synthetic local provider logs, compare cached listing and
rescan, filter by title/cwd/session ID/path, and inspect a result on the second
page. Single import must use the selected project/provider/environment and the
reviewed file hash. Batch import includes all filtered pages, skips imported
rows and reads a fresh preview for each candidate. Stop during a preview to
verify no write starts; stop during a write to verify it finishes only that one.
Simulate a lost write acknowledgement and check that reload/selection cannot
enable another import in the same sheet. Source/destination pickers and the
preview must each consume immediate Escape before the parent sheet. Check
narrow windows, both locales/schemes and originating draft retention.

### Windows publication evidence workspace

Open **研究工具 → 论文证据** inside a project. The workspace supports paper and
revision selection, hierarchical section/claim/figure/table/method/supplement
editing, multiline content and per-revision/item drafts. Returning from an editor
or switching revisions retains unsaved text for the lifetime of the page.
Missing titles and invalid ordering display validation feedback before writing.
An unconfirmed new item remains reachable through **新增条目** with its original
ID. If reconciliation finds it already saved, that entry starts a fresh draft;
local corrections remain attached to the saved item rather than creating a copy.

**添加证据** offers paged file versions, Runs and persisted message excerpts.
Messages show their sequence and text so results from the same conversation are
distinguishable. Selecting a source opens a separate binding step; **返回选择来源**
returns without registering it. A message selection uses UTF-8 byte boundaries
and the persisted content digest. **添加精确来源** also accepts message ranges,
tool calls, execution logs, code cells and registered external resources. The
host resolves exact identities inside the selected project before binding.
Evidence retains its snapshot, lineage, drift, reviews and supersession details;
draft versions can change selection to candidate, selected or rejected.

**冻结检查** uses the shared WebView readiness service. A check enables freezing
only for the same revision and policy; changes invalidate it. Freezing needs a
second explicit click. Findings and recorded waiver reasons remain visible.
Recorded waiver reasons also remain visible before a fresh check, with a reminder
to recheck after editing evidence or explanations. Escape cancels the pending
freeze confirmation and leaves the version editable.
**版本与复现** can clone a revision, display reproduction reports and build a
Capsule from a frozen manifest. Choose a directory with the system picker and a
new ZIP filename; existing files are never overwritten. Eligible evidence offers
the existing isolated reproduction verifier, with results read back from the host.

Mutations bind the selected project/revision and are never automatically replayed.
After an unconfirmed result, refresh, inspect the record and explicitly acknowledge
the reconciliation before another write. Late source reads cannot replace a new
filter, revision or closed page. Escape closes a dropdown first, then a freeze
confirmation/editor/binding step, then the parent tab/page.

Publication integration is still undergoing real-window acceptance; the overall
WinUI parity ledger, final packaged/DPI comparison and full regression remain open.

### Windows Office and scientific source quotes (acceptance pending)

Office preview selection can stage source-labelled quotes in the current chat:
DOCX uses rendered preview pages, PPTX uses slide numbers, and XLSX uses the
selected sheet/cell (or merged range), including the formula when present.
Scientific previews expose quoting through **查看源文本**: select text and use
**加入聊天** or **加入聊天并跳转**. These quotes retain source line ranges and
label unsaved draft text. Atom/residue selections on scientific canvases are
not supported by this quote path.

The native panel supplies the file and conversation identity; document markup
cannot supply a different source path. Quotes retain the existing session-owned
draft, deduplication and send/queue behavior. C# model/contract checks passed,
but browser selection coverage, a build of this latest change and real-window
acceptance remain pending at the PR submission checkpoint.

## Windows conversation readability (2026-10-05)

Completed tool phases also include a successful `attempt_completion` when the
same final answer follows in an assistant message. Successful Runs and their
exact monitors join the same process; a **查看待审阅运行** action remains outside
the fold for each Run needing review. Monitors and review-needed Runs with
unverified ownership stay visible. The sole final answer, failed tools and
active turns remain visible. Terminal Run monitor details start collapsed;
raw Run input/output is behind **原始输入与输出**, so a
review-needed status no longer expands a JSON record across the conversation.

The Windows artifacts panel projects Markdown tables from assistant messages and
successful completion output in the displayed transcript page, alongside the
registered files. Each table has dimensions, a native preview and **复制表格**
(tab-separated visible text, including headers). Collection follows streaming
updates and history navigation without creating files or database records.
Escape closes a table preview before its parent panel. CSV/formula extraction
and scanning unloaded history are outside this table projection.

The composer keeps execution environments above the input, attachment/options
on the lower left, and compact context usage beside model/effort/Fast/Send on
the lower right. **新建 ACP 对话** and **ACP 会话选项** now live inside **对话选项**.
The context button retains its accessible label, tooltip and detailed breakdown.
ACP submenus and context details still close before their parent on Escape.

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
