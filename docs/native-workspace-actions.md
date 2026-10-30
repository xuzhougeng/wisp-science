# Native workspace toolbar parity

The target is the seven WebView conversation toolbar actions, in the existing
order, implemented as SwiftUI surfaces with shared native contracts for WinUI 3.
The WebView remains supported. This work is in progress; enabled icons alone do
not establish parity.

## Latest acceptance evidence (2026-09-18)

Local Files now provides New File/New Folder and entry context-menu Rename/Delete.
The name sheet keeps new/renamed entries in the displayed directory; Delete
explicitly confirms permanent recursive removal, matching the existing WebView
backend behavior. Archived/read-only views disable the controls and the host
revalidates writable scope and session ownership. Mutations are not replayed after
an uncertain response. Successful actions refresh the same directory only while
that file view is still current. Errors keep the operation sheet open.

`native_conversation_panel_file_action` carries a typed `file_action`
(`create_file`, `create_directory`, `rename`, `delete`), `path`, optional
`new_path` (required for rename), and the explicit session/project scope. It
returns `true` after the filesystem operation and generation bump. The Rust
adapter uses the existing workspace file implementation; Swift and WinUI's
`INativePanelClient.FileActionAsync` use the same wire values. Tests cover
collision preservation, boundary/root rejection, real temporary-file operations,
name validation, scope, refreshed listings and no replay after lost responses.
Packaged interaction acceptance for these new actions is in progress.
The complete Swift suite passed 143 tests (129 UI + 14 core), C# contract tests
passed, and wasm compilation and Rust formatting checks passed. Rust native-panel
tests and the updated QA package are still running.

The packaged QA app uses a separate bundle identifier and synthetic database.
Files → README.md → Edit → Save wrote `NATIVE_SAVE_OK` to that workspace.
Escape on a dirty editor opened the discard confirmation; a second Escape closed
only that confirmation and retained the draft. Replacing the file externally
before Save produced a baseline-conflict error and retained the editor draft.
Discarding and reopening displayed `EXTERNAL_CHANGE_QA`, confirming that the
external content was not overwritten. These checks establish the local text
editor interaction; they do not establish remote editing or atomic file locking.

The cross-project inbox listed an unseen synthetic assistant reply from a second
project. Clicking it selected that project's exact session and working directory;
reopening the inbox showed no remaining entries. No model or remote host was used.

This smoke run also found a backend bug in the agent delegation read path:
an irrefutable `if let` forwarded an absent `enabled` value as a write. The branch
now requires `Some(enabled)` and the module denies irrefutable patterns at build
time. Swift regression coverage checks that both initial and quiet refreshes
perform zero delegation writes.
The rebuilt app subsequently displayed the checkbox without an error and saved
an enabled value. An authenticated scoped request read back `true`, restored
`false`, and read back `false`. The eight Swift agent-panel tests and five Rust
native-panel tests passed, as did formatting and strict packaged code signing.
After the UI toggle, automation returned `elementHasNoFrame` for refresh and
then timed out reading accessibility state; the backend remained responsive.
This intermittent automation/window interaction needs further investigation and
is not classified as either a confirmed app hang or a passed refresh interaction.
Reacquiring the app subsequently allowed the refresh action and accessibility
read to succeed with the saved false value and no error. The same intermittent
timeout recurred after HTML export; a process sample showed the main run loop,
without establishing a root cause.

Sharing two selected messages through the real macOS Save dialog produced a
9,684-byte HTML file containing the first turn and excluding the unselected
second turn. The real PNG Save dialog subsequently exported an 840 × 352 image
of those same two messages; visual inspection confirmed the selected text and
code block. Opening the Save dialog again and immediately pressing Escape kept
the Share sheet open. Broader Markdown parity remains a separate open item.
The latest full
Playwright run finished with 823 passed, two skipped and one initialization
timeout waiting for `open-session` in the 540 px completed-report test. Its
unchanged 1280/540 px tests both passed on isolated rerun. Full Rust workspace
tests are still running; the latest native-panel tests passed after the fix.

The sections below retain the implementation and verification history; older
pending statements are superseded by newer evidence, not acceptance of the
entire toolbar. The PR remains draft until the complete checklist is satisfied.

## Acceptance checklist

- [ ] Conversation outline: full persisted question index, search, historical
  navigation, repeated-prompt correctness, live updates, timestamps and timings;
  verify Escape immediately after opening.
- [ ] Share: selectable and editable/redactable user/assistant/thinking messages,
  thinking initially excluded, export preview, HTML and PNG export, PNG width
  selection. Social-copy/highlight flows are hidden in current WebView (see
  `ui/src/overlays.rs::ShareOverlay` and test-only helpers in `app_support/share.rs`).
- [ ] Trajectory: existing recorded turns, tool details, timing/token statistics,
  filters and export; no derived fake runtime data.
- [ ] Research archive: load/prepare draft, edit report/scripts, review file
  actions, explicit consent before freeze/cleanup, inspect frozen materials,
  retry cleanup and continue research. Preserve backend ownership checks.
- [ ] Needs-you inbox: current cross-project entries, badges, navigation to the
  correct project/session, seen state and refresh.
- [ ] Terminal: project/context-owned persistent PTY sessions, local and remote
  context selection, output/input, resize, interrupt, close and reconnect;
  separate native workspace authorization from ACP authentication terminals.
- [ ] Right panel: toggle and persist selected tabs; artifacts, agents, files,
  execution contexts plus the existing optional notebook, highlights, provenance
  and side-chat surfaces. Match WebView behavior and actual data sources.
- [ ] Shared DTOs, Swift and C# clients and fixtures; backwards compatibility.
- [ ] Tests for navigation races, project/session ownership, topmost Escape,
  empty/error/reconnect states; rendered desktop/narrow/dark layouts.
- [ ] Full repository verification and updated reviewable PR.

## Implemented foundation

`native_conversation_outline` validates the project/session owner, flushes saved
conversation events and returns the full persisted question index. Each entry
contains its global question index, text, timestamps and the next question's
exclusive transcript cursor. Snapshot `user_offset` is additive and allows a
client to locate identical prompts without text matching. Older hosts omit this
field; native navigation reports a refresh/upgrade error instead of guessing.
SwiftUI has a searchable outline popover and history navigation. WinUI now
hosts the live conversation loop plus the same outline, inbox, trajectory,
archive and HTML share pages, a text terminal pane, and a right-hand panel
with file mutations, hosts, agents, notebook stars, highlights and side-chat.
PNG export and VT rendering remain follow-ups. Remaining acceptance items
above still apply.

## Initial foundation verification (historical)

The focused Swift conversation and outline suite passes (11 tests), covering
history cursor routing and global question indexes for repeated prompts. The
C# contract executable passes with the shared outline fixture. Rust check and
DTO tests are running; full checks, Escape interaction testing and visual QA
are still outstanding. No completion or full parity claim is made yet.

## Trajectory and inbox implementation

The trajectory button opens a native sheet with recorded turns, searchable
inputs/outputs, token and timing statistics, a step inspector and HTML export.
`native_conversation_trajectory` uses the existing trajectory fold;
`native_conversation_trajectory_html` uses the same HTML renderer as WebView,
while SwiftUI owns the save dialog. C# exposes both operations. The current bar
visualization still needs comparison with WebView's lane/time-axis behavior;
visual equivalence and immediate topmost Escape have not yet been verified.

The inbox button shows the same 50-session cross-project search and needs-you
filter as WebView, polls every 20 seconds, refreshes on opening, and navigates
using both project and session IDs. `native_conversation_seen` validates ownership
and marks a successfully opened session seen. Failed reads do not mark it seen.
C# exposes inbox and mark-seen operations with the existing SessionSearchInfo
shape. Live app interaction and badge refresh after opening still need smoke QA.

Latest focused Swift run: 15 passed, one opt-in render test skipped. Coverage
includes repeated prompt history positioning, trajectory scope/search/export,
inbox failure preservation and marking only successfully read sessions seen.
Rust compilation remains running; it must be rerun for the final changed tree.
The right-side panel is enabled for artifacts and files; its other tabs remain pending.


## Research archive implementation

SwiftUI now loads/prepares the existing archive draft and supports report/script
editing, file action selection, explicit consent, immutable freeze and cleanup,
cleanup retry and continuation. Editing clears consent. A failed confirm reloads
the saved record once without replaying a mutation, including when cleanup failed
after a successful freeze. Saved materials open in a child Quick Look preview;
canonical paths must remain inside the selected workspace. The existing backend
still verifies ownership, file checksums, writable scope and deletion eligibility.
The C# `INativeArchiveClient` uses the same ResearchArchive and confirmation DTOs.

Archive tests cover scope, confirmation fields, consent invalidation and failed
cleanup reconciliation/no replay. Three behavior tests passed; the opt-in renderer
also passed and desktop/narrow/dark output was inspected. A missing explicit
surface background was fixed during visual inspection. Final preview integration
compiled and behavior tests passed again. Actual Escape/topmost interaction and
real app smoke remain unverified; no live user research was archived during QA.
C# shared fixture verification passed. The initial Rust check completed, and a
second Rust check including archive changes is running; full CI remains pending.


## Share implementation

The native share sheet now loads full history in bounded transcript pages,
selects user/assistant rows by default, excludes reasoning by default, supports
editing the export copy, select-all/none, case-insensitive keyword masking,
preview, width selection (320–2400, default 840), and HTML/PNG file export.
Only edited/redacted selected rows are submitted for HTML generation. The host
validates session ownership and renders HTML using the existing standalone
WebView export stylesheet with escaped raw HTML and restricted link schemes.
PNG uses the actual SwiftUI preview at one pixel per requested width unit.
Images over 40 million pixels or 32768 pixels tall produce an explicit error
with HTML as an alternative; transcript sharing is capped at 16 MiB.

The original acceptance draft incorrectly included social-copy/highlight flows:
source inspection shows these are test-only/hidden in the current WebView.
Visible ShareOverlay has PNG and HTML export only, which remains the target.

Four focused Swift tests passed, including redaction/default selection, width,
Markdown block preservation and actual PNG pixel dimensions. The redacted PNG
was inspected. C# fixture/client compilation passed. Native PNG currently covers
paragraphs, headings, bullets, quotes and fenced code; tables and complex nested
Markdown layout still need alignment and must not be considered complete.
Rust share-render tests are running. Full runtime/Escape smoke and all-suite
verification remain outstanding.


## Terminal implementation

The terminal button now opens a resizable native dock. An empty dock opens the
local execution context once; existing PTYs reappear on reopening. Additional
terminals use registered execution contexts (local/WSL/OpenSSH through the
existing launch implementation). Hide detaches the view without closing the
process. Closing a terminal tab terminates and unregisters it.

The authenticated native commands validate project and frame scope on every
operation, exclude ACP authentication terminals, and require writable scope for
open/write/resize. Reads return base64 raw bytes with absolute start/end cursors;
bounded scrollback resets are explicit. The frontend uses SwiftTerm 1.19.0's
AppKit control for VT/ANSI, selection, keyboard and alternate-screen behavior.
Input is serialized, never automatically replayed, and uncertain input pauses
queued bytes until the user explicitly resumes. C# INativeTerminalClient exposes
the same raw-byte boundary for a future WinUI terminal control.

SwiftTerm is pinned by Package.resolved and its MIT notice is included in the UI
resource bundle. Packaging copies its resource bundle and adds the SwiftPM lookup
symlink. Xcode's Metal Toolchain component is required by its packaged shaders;
install via `xcodebuild -downloadComponent MetalToolchain` when absent. The local
component was installed and the default Swift build then passed; the temporary
native-build-system workaround is not required by project scripts.

Three focused Swift tests passed: byte cursor validation, no replay of uncertain
queued input, and real native emulator cursor/alternate-screen handling. The ANSI
output bitmap was inspected. C# fixture/client verification passed. The scoped
native PTY lifecycle Rust test passed. Reconnect under high-volume rollover,
live app keyboard/resize, cross-platform runtime smoke and full-suite checks are
still pending; SSH/WSL tests must continue to use mocks, not real hosts.


## Side-panel foundation

The seventh toolbar action now toggles a native, width-adjustable panel and
persists visibility and selected tab. Artifacts and local files are connected:
name filtering, directory/parent navigation, file sizes, scoped artifact reads,
text and Quick Look previews, truncation notices, and refresh. Quick Look is
shared with archive material previews. No preview executes a file.

`native_conversation_panel_*` resolves the requested frame to its actual working
project and state scope, validates artifact visibility, and reuses existing
file boundary/preview logic; it never depends on a shared hidden window's current
frame. Swift/C# use existing ArtifactInfo/DirEntry/FileContent shapes. Focused
Swift tests for late directory responses and truncated preview metadata and C#
fixtures passed. The Rust file-boundary integration test passed after adding
the missing tempfile test dependency.

This is not side-panel parity yet: agent workflows, advanced execution controls, optional
notebook/highlights/provenance/side-chat tabs, tab add/close/reorder state, remote
file operations, editing/download/actions and full visual/Escape QA remain to be
implemented/verified. Their absence must not be treated as task completion.


## Session execution contexts

The native right panel now includes Execution Environments. It shows the local
context plus contexts attached to the current conversation, with status, probe
errors and expandable machine capabilities. Available contexts can be attached;
nonlocal contexts can be detached. Probe uses the existing settings command and
runs only on explicit user action. No test contacts a remote machine.

The shared `PanelContexts` response uses the existing ExecutionContext fields,
conversation membership and a read-only flag. Membership mutations validate
project/session ownership, existing context identity, archive state and writable
scope. Swift and C# clients share `panel-contexts.json`; uncertain writes are not
replayed. Read-only views disable attachment changes. A reply arriving after
navigation cannot trigger a stale context refresh.

Swift behavior tests cover session filtering, read-only controls and mutation
failure without replay. C# contract tests passed. The 280-point light/dark cards
were rendered and inspected. The latest full native suite passed (60 tests,
three opt-in render tests skipped); all five panel tests also passed with
rendering enabled. The Rust file-boundary integration test and ten shared native
DTO tests passed. Formatting checks passed. Full Rust workspace, WebView and
packaged-app verification remain pending.

Runtime management, run lists, interpreter/storage editors and context terminal
shortcuts still need to be integrated in this panel. Existing native settings
remain available for configuration. Agents, optional tabs and tab management
remain pending; this addition does not complete the side-panel acceptance item.

## Runtime and run activity panels

Execution-context cards now open native runtime and run sheets. Runtime rows show
language, status, interpreter/version, process memory and last error, with variable
inspection. The backend reuses WebView's runtime visibility predicate: the active
conversation and shared runtimes are visible, other mainline projects remain
visible for resource awareness, and exploration scopes exclude foreign sessions.
No hidden window's active frame is used.

Run lists reuse scoped RunSummary data and load only the selected RunRecord's
command/output/error details. Lists poll while open. Users can cancel a live run
with confirmation or retry harvesting a successful run's outputs. These commands
use the existing RunManager, require an unarchived writable conversation and
reject mutation of inherited or foreign-scope runs. No mutation is retried on a
lost reply. A failure stays visible across polling; dismissing details invalidates
late replies. Refresh does not flash a full loading indicator on every poll.

`PanelActivity` reuses wisp-dto RuntimeInfo/RunSummary, preserving the runtime
camelCase wire format and the run snake_case format. Swift and WinUI-facing C#
clients share activity, run-detail and variable fixtures. All 63 native tests
passed with opt-in rendering enabled; C# contract checks and 11 native DTO tests
passed. Runtime/list/detail screenshots were inspected. Both Rust panel tests
passed, including rejection of foreign-project and missing run IDs for reads and
mutations. The three reused runtime-visibility tests also passed.

Remaining within this area: runtime start/stop/restart, console execution,
interpreter/storage controls, full run workspace/file review and cleanup, and
live topmost-Escape interaction verification. These sheets are not yet full
WebView runtime/run parity. All other outstanding toolbar acceptance items remain
in scope.

## Runtime lifecycle and console

The runtime sheet now starts Python/R, stops or restarts an exact displayed
runtime, dismisses dead runtime records and runs code in the current
project/conversation/context. Stop/restart require explicit confirmation because
they discard interpreter state. Lifecycle requests include the displayed runtime
generation; the host rejects a stale generation at dispatch. Native stop targets
one runtime ID rather than accidentally stopping every conversation in a foreign
project. Read-only sessions cannot start/restart/execute, but can still stop an
existing runtime and release its resources.

Console execution reuses RuntimeManager, the code-size limit, local exploration
source checks, existing output formatting, plot output and scope-generation
updates. It never auto-replays an uncertain execution. Execution and stop have
independent in-flight state so the user can stop a busy interpreter. Runtime
requests and execution results have matching Swift/C# APIs; the execution fixture
uses the existing shared RuntimeExecutionSummary DTO. No new execution engine,
WebView dependency or real remote-host test was introduced.

All 66 native tests passed with render tests enabled, including the console
render. C# contracts, 11 shared DTO tests, three runtime visibility tests and
three native panel tests passed. Formatting checks passed. Live keyboard/Escape checks,
interpreter/storage controls, script binding, task file review/cleanup and the
remaining right-panel tabs still need completion and end-to-end verification.

## Agent workflow inspection

The default Agents panel now lists the current conversation's persisted workflows,
including nested workflow ownership and the root conversation when viewing a
taken-over child. It uses the existing delegation snapshot loader rather than a
new workflow store. Lists refresh while visible. Tasks show dependencies, executor,
tools, approval reasons, summary/error and token/tool usage; persisted results are
loaded on demand using workflow and stored-step identities.

Result sheets match WebView's response-envelope handling: structured summary and
diff, files, artifacts, persisted evidence, tests, risks, extra fields and errors.
Artifacts are deduplicated by identity, persisted evidence takes precedence, and
raw JSON remains available as a secondary disclosure. Markdown content is rendered
natively. Result requests validate conversation membership before reading; Swift
and C# also reject mismatched workflow/step replies. Closing the surface drops
late responses. Background polling does not clear a result error.

Swift/C# interfaces and shared fixtures preserve workflow version, root/parent
identity, editable proposal, task metadata and full result data for subsequent
native editing/launch integration. Seventy native tests passed before the final
result-card styling pass; focused agent tests cover that pass. C# contract tests
and twelve shared DTO tests passed. Rust conversation/root-frame snapshot checks
passed. Light/dark result sheets and a 300-point panel were rendered and inspected.

Agent creation, plan editing, delegation enablement, approve/run/cancel/discard,
retry budgets, specialist controls and workflow-template operations remain to be
connected. This is inspection parity only; it does not complete the Agents panel
or the overall toolbar acceptance checklist.

## Agent workflow controls

Root workflows now expose draft approval/discard, approved-plan execution,
running cancellation and failed/cancelled retry. The native approval carries the
reviewed version; the backend keeps the existing atomic version/status check.
Approval, execution and retry share extracted project-explicit handlers with
WebView, so native requests do not depend on a hidden window's active frame.
Retry supports per-task token overrides, including zero for unlimited, and uses
the existing policy/budget validation. Automatic workflows retain automatic
launch behavior. Legacy skill-bound workflows remain blocked by the existing
conversion rule rather than bypassing it.

Mutations validate conversation membership and root-workflow ownership. Archived
or non-writable conversations cannot approve/run/discard/retry, while running
workflows can still be cancelled. Full proposal details, task grants and budgets
are available for review. Destructive actions and approval use native confirmation
surfaces; retry has a budget editor. In-flight launch and control requests are
tracked separately, so cancellation remains usable during execution. Polling does
not invalidate a pending action's error, and uncertain actions are never replayed.

Swift and C# share the closed action set and reviewed-version/budget contract.
All 73 native tests passed before the final plan/retry-layout additions; the
focused seven agent tests passed afterward. C# contracts and thirteen native DTO
tests passed. Native panel Rust tests and both shared retry preservation tests passed.
Formatting checks passed. Approval and retry layouts were rendered and inspected.

Subsequent delegation and workflow-settings navigation work is recorded below.
Immediate-Escape interaction tests, tab management and other outstanding toolbar
acceptance requirements remain open. No real provider, agent execution, SSH or
WSL service was contacted during these tests.


## Delegation and workflow management entry

The Agents panel now reads and saves the current session's delegation setting.
The switch changes only after the backend confirms the saved Boolean; failures
retain the previous value and are never automatically replayed. Reads omit
`enabled`, while writes preserve explicit `false`. The backend validates session
ownership, rejects writes to archived/read-only sessions and reuses the existing
session-specific delegation setter. Swift and C# expose the same operation.

Manage Workflows opens native Settings directly at the `workflows` section,
retaining the current project. It clears any prior project-editor route; opening
project settings in turn clears this section route. The existing native workflow
settings provide template add/edit/copy/delete and conversion entry points.

Scope correction: `ui/src/agent_workflows.rs::agent_workflows_panel` routes its
Manage Workflows action to settings. Its editor saves workflow templates via
`save_workflow_template`; it does not expose direct creation/editing of session
plans. Earlier pending references to a separate session-plan editor were an
incorrect inference, not a WebView parity requirement. Template-editor visual and
interaction fidelity still needs review alongside the other settings surfaces.

Verification: all 75 Swift tests passed with render tests enabled, including
confirmed switch state, failed-save no-replay and settings-route isolation.
C# contracts, fourteen shared DTO tests and three native-panel backend tests
passed. The narrow Agents panel render was inspected. Full repository gates and
live UI/Escape verification remain outstanding.

Manual smoke for this increment:
1. Open Agents in a writable conversation, toggle delegation, and reopen the
   panel to confirm persistence. Confirm a read-only conversation disables it.
2. Select Manage Workflows and confirm Settings opens at Workflows for the same
   project; close it and open project settings to verify the project editor.
3. Simulate a save failure and confirm the displayed value stays unchanged and
   no second save request is issued automatically.

## Native right-panel tab strip

The native panel now uses a horizontally scrollable tab strip instead of a
single picker. Each implemented tab can be closed, reopened through Add Panel,
dragged to a new position, or moved left/right through its native context menu.
Closing the selected tab picks its left neighbor (or the first remaining tab),
matching `close_right_tab` in WebView. Closing the last tab collapses the panel;
opening it again restores the default tabs. Selecting an offscreen tab scrolls
it into view. Order and selection persist in local native preferences.

Swift `NativePanelTabs` and C# `NativePanelTabs` expose the same stable IDs,
restoration, selection, close, reorder and reopen operations. This is local UI
state, not a new backend command or persistence table. Unknown/duplicate saved
IDs are filtered and malformed preferences fall back to defaults. The registry
includes Notebook, Highlights, Provenance and SideChat for later data-backed
integration; the visible menu currently contains the four implemented panels.
Those optional surfaces remain required follow-up work, not completed parity.

The close glyph is exported from the existing shared `compose_icon` set.
Native icon NSImages are marked as templates so AppKit menus tint them correctly
in dark mode. Narrow light/dark tab strips were rendered and inspected; the dark
menu-icon regression found during inspection was corrected and rerendered.
All 79 Swift tests and the C# contract executable passed before that final visual
correction, and the focused render test passed afterward. Tests cover restoration,
corrupt preferences, left-neighbor selection, last-tab reopening, bidirectional
moves, deduplication and opting optional surfaces into the registry. Generated
native design resources pass the sync check.

Manual verification still required: drag tabs in a real window, close/reopen the
last tab through the toolbar, restart to verify the saved order, and press Escape
immediately after opening Add Panel or a tab context menu. Full repository gates
remain pending until the remaining toolbar implementation is ready for review.

## Provenance panel

Add Panel now includes Provenance with a live tool-call count. It projects only
`role: tool` entries from the displayed conversation page, matching WebView's
`ProvenancePane`: successful calls start collapsed; failed/incomplete calls start
expanded. Each row displays recorded tool name, input, output and tri-state
result without inventing an execution status. Empty input/output sections are
omitted. Text remains selectable, and search covers tool names and both bodies.

The panel follows the same latest/history page as the conversation. It consumes
the existing scoped conversation snapshot rather than fetching another snapshot
or adding a separate store. A page change resets disclosure state; live updates
on the same page preserve manual disclosure choices. Swift and C# expose matching
`NativeProvenanceRow` projections over the existing shared transcript Item DTO.
The shared fixture covers success, failure, incomplete execution, absent input
and non-tool messages. Rust verifies that fixture against the actual DTO.

All 83 Swift tests (rendering enabled), C# contracts and fifteen shared native
DTO tests passed. Narrow light/dark
provenance renders were inspected. Projection tests verify exact text, source
order, default disclosure, search and replacement of another transcript page.
The native model test confirms this tab never dispatches a file read or duplicate
snapshot request. Live scrolling/history/disclosure interactions remain part of
final native smoke verification. Notebook, Highlights and SideChat remain open
implementation items, alongside the other toolbar acceptance work above.

## Saved text highlights

Add Panel now includes Highlights, backed by the existing app-global library's
text snapshots for the requested project and conversation. Cards expose copy,
remove-from-library and reveal-in-conversation actions. Removal does not edit the
source transcript and remains available for archived/read-only sources, as in
WebView. The native broker validates the source project, session and `text` kind
before deleting; unrelated code/figure snapshots cannot be removed through this
route. Responses reuse the existing shared LibraryItem DTO. No new database or
secret storage was added.

Swift validates returned source identities before showing rows, and C# exposes
matching list/remove methods with the same validation. A failed/uncertain removal
retains the displayed card and is never replayed automatically. A late list read
cannot restore a successfully removed card. Search filters excerpt text.

Reveal searches the currently loaded transcript, ignoring whitespace like
WebView's saved-mark navigation. It scrolls to the first matching rendered message
and briefly highlights the matching text; explicit navigation disables automatic
following of new replies. Missing text reports that the corresponding history
page must be loaded. Swift returns character offsets and C# UTF-16 ranges suited
to each platform's text controls. Persistent underlining of all saved excerpts,
selection-based creation from the native message context menu, and richer tool
body reveal remain follow-up work; this increment connects the saved-excerpt
panel rather than claiming every transcript selection interaction is complete.

All 88 Swift tests with rendering enabled and the C# contract executable passed.
Focused tests passed again after tightening scope validation and clearing marks
on history navigation. Narrow light/dark highlight cards were rendered and
inspected. Tests cover scope mismatch, no replay, late reads after deletion,
Unicode/whitespace matching, first occurrence and highlight expiration guards.
All four backend panel tests and sixteen shared DTO tests passed. Backend tests
exercise foreign project/session rejection and preservation of non-text library
items using a temporary database. Full repository gates and
live clipboard/history/selection smoke tests remain pending.

## Notebook panel

Add Panel now includes Notebook with a code-cell count. The panel projects the
same displayed transcript as WebView: assistant fences (excluding CSV/TSV/FASTA
bodies), Python/R runtime calls and shell calls. Executed code replaces matching
assistant fences after trim comparison, while repeated executions remain separate
cells. Runtime context prefixes are removed only from Python/R previews. Cell
language, source, output, origin and tri-state status remain intact. Failed output
starts expanded; other output starts collapsed. Code and output are selectable,
copy preserves the source, and search covers language/source/output.

The shared `panel-notebook.json` fixture is checked by the real WebView
`collect_notebook_cells` implementation and by Swift/C# projections. It covers
executed-fence deduplication, repeated executions, data-fence exclusion,
unspecified language, unclosed fences, runtime prefixes and shell bracket syntax.
This provides an executable reference for keeping both native implementations
aligned with WebView. Per-cell disclosure state resets when a live projection
replaces the cell with different code/origin.

Code stars reuse `star_library_code` and the existing LibraryItem contract.
Native list/delete operations validate project, session and `code` kind, sharing
the library scope guard with Highlights. Source archives remain immutable;
collection edits still work for read-only conversations. Swift and C# expose
matching list/star/unstar operations. Failed mutations are not automatically
replayed or optimistically displayed. Late list results cannot remove a confirmed
star, and repeated execution cells share the same in-flight star identity.

All 92 Swift tests passed with renders enabled before the final identity guard;
five focused notebook tests passed afterward, including mismatched reply rejection.
C# contracts, three WebView notebook tests, four native backend panel tests and
the wasm32 frontend check passed. Narrow light/dark renders were inspected.
Native syntax coloring, live clipboard/disclosure testing, SideChat and the
remaining toolbar acceptance work are still pending. Notebook does not introduce
a new run-code action: the existing WebView panel also exposes copy and library
controls, while execution stays in the conversation/runtime flows.

Manual smoke: add Notebook; inspect source versus executed cells; expand successful
output and verify failed output starts open; copy code; star and unstar a repeated
cell and confirm the other copy reflects it; switch historical pages; reopen the
panel and confirm library state. Repeat with an archived conversation to verify
library edits do not modify its transcript.

## Side chat

The final optional tab, SideChat, now has a native question composer, model/ACP
picker, read-only quote cards, answer rendering, evidence disclosure, no-evidence
state, errors and session-local clear. Evidence retains source IDs, turn numbers,
event/message locators, relevance and snapshot version from the existing backend.
Questions reuse `side_chat` and its frozen evidence retrieval; they never become
main-conversation messages. The HTTP picker changes the existing global active
model just as WebView does; ACP selection belongs to the side-chat model.

The native broker validates the requested conversation and the returned session
identity. Shared SideChatResponse now preserves the backend's additive sessionId
field (optional for old payloads); Swift/C# require it for a native reply. Model
choices are filtered with the existing Rust `is_chat_model` logic. ACP working
directory resolution now uses the requested frame's working project, avoiding an
unrelated hidden window's active root and preserving exploration directories.

The native session cache is keyed by database, project and session. Closing a
panel or switching sessions does not redirect or discard a pending answer; busy
state belongs to that same session. Requests and failed model changes are never
automatically replayed. An uncertain HTTP-model selection disables sending until
an explicit model-state read reconciles it. Quote source labels are escaped and
questions use read-only blockquotes, without main-composer file-edit instructions.
C# exposes typed options, model selection, question/reply and quote formatting.

All 99 Swift tests passed with rendering enabled before the final model-selection
guard; focused side-chat/navigation tests cover that guard. C# contracts, seventeen shared DTO tests, the wasm32 frontend check and
sixteen existing backend evidence/classification tests passed with fake providers.
Narrow light/dark renders were inspected. No real provider or ACP agent was
contacted. Return/Shift-Return handling is implemented in the following increment. Direct
text-selection quote integration, expanded source rendering and immediate Escape
on the model menu still require live UI verification/alignment. Full repository gates and the overall toolbar acceptance
audit remain open; exposing all eight tabs does not establish complete parity.

Manual smoke: ask about the current conversation, inspect source evidence, add and
remove a quote, choose HTTP and ACP models, switch sessions while a reply is
pending, reopen the panel, and verify the answer remains with its original
session. A failed request should expose an error without making a second call.


## Side-chat keyboard and input methods

The native side-chat composer now uses an AppKit text view hosted in SwiftUI.
Return sends, Shift-Return inserts a newline, and Return during marked-text
composition stays with the input method. Busy/empty Return is consumed without
adding a newline or submitting again. Command-Return is handled locally when this
editor owns focus; the side-chat send button no longer registers a competing
window-wide shortcut. An unchanged draft preserves the user's selected range,
and external updates do not replace marked text mid-composition. Native callbacks
are removed when the editor is dismantled.

Swift and C# expose the same return-key decision for WinUI integration. All 106
Swift tests (including rendering) and the C# contract executable passed. Six new
AppKit tests directly dispatch key events to a text view in a test window, covering
Return, Shift-Return, marked-text confirmation, busy/keypad Return, local
Command-Return and selection preservation. A focused render with a multiline
draft passed afterward, and its narrow dark layout was inspected. The complete
application's shortcut routing with a real IME still belongs to final live smoke
verification; these tests do not replace the window-level Escape audit.

## Native message selection and saved marks

Ordinary conversation message bodies now use a selectable AppKit text view hosted
in SwiftUI. The native context menu adds Quote to Side Chat and Save Highlight to
nonempty selections, alongside standard text actions. Each menu action captures
both the selected text and its callback at menu creation, so a streaming update
cannot silently replace the selected excerpt. The workspace validates the
original project/session before opening SideChat, adds a read-only quote and
reveals that tab without sending a question automatically.

Save Highlight reuses `star_library_text`, with the existing source metadata,
size validation and deduplicated library storage. The native conversation model
validates the returned text and source identity and applies it only after success.
Duplicate in-flight saves are suppressed; errors are not automatically replayed.
A late result or an old menu cannot mark a different session. WinUI's typed
highlight client now exposes the same save operation.

Saved excerpts underline every whitespace-insensitive occurrence in rendered
message text. Swift character ranges and C# UTF-16 ranges preserve their native
text-control indexing. Markdown emphasis, links and configured UI fonts remain
available, and mark updates preserve the selected range. Removing a highlight
from the panel removes only that item's mark, so a stale panel list cannot erase
an unrelated newly saved excerpt. Message-open reads and save responses have
independent generation guards. A successful save refreshes an open Highlights tab.

New tests cover frozen menu selection/callbacks, Unicode, repeated marks,
formatting, selection preservation, failed/mismatched saves and session switches.
Narrow light/dark selectable-message renders were inspected. These tests use
actual AppKit text selection and menu action objects; they do not yet prove the
complete application's immediate-Escape behavior for a displayed context menu.
File-preview selections and tool-input selections still need the same native action wiring.
Full repository gates and final live toolbar smoke verification remain pending.

Validation: all 114 Swift tests (including opt-in rendering), 17 DTO contract
tests, four native panel backend tests and the C# contract executable passed.
The first Swift run hit a 30-second timeout in the existing process-transport
test; its focused rerun and the subsequent complete suite passed without changing
the transport timeout. The cause of that transient failure is not established.

## Tool-output selections

Tool result bodies share the native quote/save selection actions and persistent
marks. Tool text remains literal and monospaced, including Markdown-like syntax.
Revealing a saved tool-output excerpt expands its disclosure and keeps it open
after the transient reveal highlight clears. Disclosure state resets on session
or history-mode switches. All 115 Swift tests passed, including the new literal
Unicode tool-output selection test and opt-in renders. Tool inputs retain their
existing selectable display; custom input actions are not yet wired.

## File-preview quotes

Text previews for files and artifacts expose Quote to Side Chat with the original
path attached as the reference source. Quoting closes the preview and selects
the SideChat tab without sending a request. The source preview must still be
open and contain the selected text, and the receiving side chat must match the
project/session. Dismissed previews and mismatched paths are rejected. Binary
previews keep Quick Look. Text previews wrap to the available width. File excerpts
do not offer transcript-highlight saving, since they are not conversation text.

The 103 UI tests passed, including preview-source validation and quote-only menu
coverage. In this run the same existing process-transport test timed out again
in the separate 14-test core target; this run is not a full-suite pass. The
previous 115-test run passed. Full repository checks remain in progress.

Preview validation additionally covers light/dark rendering with explicit native
appearance and a solid sheet background; both images were inspected. The focused
panel/selection suite passed 14 tests, followed by a passing configured-code-font
regression test. Tool and preview text honor the code font family and size.
A test-only invalid palette token initially crashed the new snapshot; it was
corrected to the existing `bg-elev` token before the successful rerun.

During full gates, a sample of the Rust `project_queries` test process showed it
at `_dyld_start` with a 96 KB footprint before it later ran and passed all seven
tests. This establishes a process-start delay in that run; it does not establish
the cause of the Swift transport timeout. No production timeout was changed.

## Trajectory timeline and inspector alignment

The native trajectory now uses the same three input/model/tool lanes as WebView,
with session-level duration, turn and call packing. Usage rows remain in the
step list but are excluded from the lanes. Missing or nonpositive durations use
the same fallback display weight as WebView; they are not reported as measured
elapsed time. Turn groups show the recorded input/model/tool timing split.
Selecting a lane segment opens the inspector and scrolls to the matching row.
Selections use stable turn/cell keys and resolve their content from the newest
snapshot, rather than retaining an old copy of the cell. Filtering reconciles
selection to the first remaining row. The inspector exposes summary, preview,
raw JSON and source views; errors include explicit `ok: false`, and the current
conversation running state distinguishes pending from running cells.

Shared fixture `trajectory-layout.json` covers all three axes, usage exclusion,
missing/negative durations, nonconsecutive turn identities, case-insensitive
trimmed filtering, empty matches and per-turn timing. The actual WebView layout
functions and the Swift/C# projections consume it independently. Nine WebView
trajectory tests, the C# executable, four focused Swift tests and a full 123-test
Swift run passed. The existing transport timeout did not recur in that full run.
Full trajectory and lane-only light/narrow and dark/desktop renders were
inspected. The final tool-preview adjustment is covered by a focused rerun.

Trajectory colors and four shared icons now export from the existing WebView
CSS and `compose_icon` sources through `sync_native_design.py`, including custom
native palette variants. Export consistency passes. Live in-flight event
projection beyond the persisted snapshot polling, real chart click/scroll
interaction and immediate topmost Escape still require final app verification.
The workspace Rust gate is still running; this section does not claim the full
seven-action objective is complete.

Repository gate checkpoint after trajectory alignment: `cargo fmt --all --
--check`, UI formatting, generated native-asset consistency and the wasm UI
check passed. The 826-test Playwright run finished with 823 passed, two optional
real-Motif tests skipped and one timeout in the actual Chromium extension-manager
tutorial screenshot. The failed screenshot passed its isolated one-worker rerun
in 3.2 seconds with no test or timeout changes. This is an initial failure plus
a successful rerun, not an uninterrupted all-green run. Generated research-
journey screenshots from the suite were restored; they are unrelated to this PR.
The workspace Rust test process remains live and must be collected before final
acceptance. Other native toolbar interaction and remaining functional checklist
items above are still open.

## Window-level native Escape stack

Native overlays now share one application event monitor and an ordered,
window-scoped registration stack. Within the event's key window the newest
presentation alone receives Escape; disabled top overlays consume it rather
than exposing their parent. Held-key repeats cannot close subsequent layers.
Menu tracking, modal windows, attached sheets and marked IME text retain their
native handling before the overlay stack runs. Registrations weakly reference
the coordinator/view, update their callback without changing order and remove
the shared listeners when the last overlay disappears.

Five AppKit routing tests pass: immediate topmost dismissal without moving
focus, disabled/repeating keys, distinct windows and updated callbacks, menu and
IME precedence, and detached/deallocated overlays. They exercise the exact event
routing method with real NSWindow/NSEvent objects and explicit window context;
they do not replace live menu/popup/sheet keyboard smoke verification. The full
Swift run passed the 114 UI tests, but both existing shell-process transport tests
hit their 30-second timeout in the separate core target. The transport is being
diagnosed without relaxing the production timeout. Packaged app build is running
for subsequent live QA; no completed-build claim is made here.

The seven-test process-client suite passed its subsequent isolated diagnostic
run (0.858 seconds), so that attempt did not reproduce a running child to sample.
The intermittent full-run timeout remains unexplained; no transport behavior or
timeout was changed. An empty/loading trajectory has no visible inspector, so
its first Escape now dismisses the sheet instead of consuming an invisible
inspector state.

Two additional tests host the actual `NativeTrajectoryView` in an NSWindow and
route Escape through its registered stack immediately after layout without
moving focus: an empty trajectory closes on the first event; a populated
trajectory closes only its inspector on the first event and its sheet callback
on the second. Both passed. These prove the SwiftUI-to-stack wiring and avoid
an invisible inspector consuming Escape, while real native menu tracking and
application event-loop smoke remain pending.

## Isolated toolbar fixture

`cargo run -p wisp-store --example native_toolbar_fixture -- /tmp/new-toolbar-qa`
creates a new synthetic store through the normal schema migrations. The target
parent must exist and the target directory must not exist; an existing target,
including a symlink, is rejected before opening SQLite. It creates two projects,
35-turn conversations with repeated questions and a recorded Python tool result,
named empty conversations, Markdown/CSV artifacts and real local preview files.
It does not configure a provider, load credentials, send model requests or run
the recorded code.

For manual native QA, the backend host must use an isolated application identifier
and its app-data directory must resolve to this fixture root. Set
`WISP_BROWSER_DATABASE` to the fixture's `wisp.sqlite` when launching SwiftUI.
Changing that variable alone does not redirect the backend: its descriptor must
match the selected database. Do not point an ordinary production host at the
fixture or copy synthetic rows into a user's research database. The isolated
host/package wiring and live smoke results still need verification.

Fixture execution passed: two projects, four frames, 142 messages and four
artifacts were created. Re-running against that directory failed before opening
SQLite and preserved the database hash. The real read-only `wisp-service`
returned both projects, only the selected project's two sessions and a paginated
long transcript. The fixture uses no model or remote environment.

## Packaged resource signing correction

The first complete package build failed at codesign because the legacy
`SwiftTerm_SwiftTerm.bundle` symlink at the `.app` root was unsealed content.
SwiftTerm 1.19's renderer already probes `Contents/Resources`, and the current
SwiftPM accessor does too. The builder now removes that old symlink when present,
keeps the bundle (including `default.metallib`) in `Contents/Resources`, and runs
strict recursive signature verification after signing. Removing the stale link,
re-signing the assembled app and `codesign --verify --deep --strict` all passed.
A complete script rerun is underway; terminal rendering in the launched package
is still part of live QA. This supersedes the earlier root-symlink workaround.

Review is available as draft PR #1288. It explicitly retains the remaining
functional and verification checklist; draft creation does not mark parity
complete.

The full ordinary packaging script rerun completed successfully, including
strict recursive signature verification. A separate `--qa` build mode is now
being validated: it uses `target/native-macos-qa/Wisp Science QA.app`, a distinct
SwiftUI bundle identifier and backend identifier
`science.wisp-science.native-toolbar-qa`. Its backend data is isolated from the
ordinary application. Launch it with `WISP_BROWSER_DATABASE` explicitly set to
the synthetic database; the standard SwiftUI default path is unchanged.
The QA app-data `wisp-science` directory may be a newly created symlink to the
fixture root, provided no preexisting data is replaced. Building the QA app and
verifying its descriptor/database identity must precede interactive tests.

## Isolated packaged QA results (2026-09-18)

The QA build completed and both bundle identifiers and strict recursive signatures
were verified. Before opening any conversation, the authenticated host descriptor
was checked against the synthetic database's canonical path. Backend startup also
adds a default workspace; manual actions were limited to `toolbar-qa`.

Live checks in the packaged SwiftUI app passed:

- Open the 35-turn synthetic session and its question outline. Press Escape
  immediately: only the outline closes.
- Open trajectory, click the final Python tool lane: the list scrolls to turn 35
  and the inspector shows the matching recorded tool output. Press Escape once:
  the inspector closes and the trajectory sheet remains. Press again: the sheet
  closes. Neither Escape check first moves focus into the inspector.
- Open terminal: a local terminal is created in the synthetic project directory.
  SwiftTerm renders its prompt and accepts `printf 'NATIVE_TERMINAL_OK\\n'; pwd`;
  both the marker and the synthetic working directory appear. Hide and reopen:
  the same terminal and its previous output remain visible.
- Open Share: all 70 shareable messages load; immediate Escape closes it.
  Open Archive with no configured provider: the existing missing-API-key error
  appears and destructive confirmation stays disabled; Escape closes it. This
  verifies the failure path only, not model-backed archive generation. Open the
  empty Needs-you inbox: its empty state loads and immediate Escape closes it.
- Open the side panel: both synthetic artifacts appear; Execution Contexts shows
  the local context with probe/runtime/run-list actions. Those actions and terminal
  resizing/high-volume output still need separate live checks.

The full Rust workspace suite finished successfully, including all 917 Tauri
tests. This supersedes earlier notes that the workspace run was still pending.
The full Playwright run had 823 passes, two optional skips and one extension-page
screenshot timeout; that exact test passed unchanged on isolated rerun. These
results do not establish complete seven-action parity; PR #1288 remains draft.

## File and artifact display modes

File and artifact panels now share a persisted client-local list/grid preference,
matching the WebView's shared view-mode control. Grid columns adapt to sidebar
width, while long names and paths remain bounded with the full text available in
help. Both presentations retain the same scoped data and preview action. The
artifact context menu also opens preview or the existing Provenance tab, reopening
that tab if it was previously closed, as the WebView menu does.

An offline render test exercises both modes at 320 px sidebar width in light and
dark appearances. All four resulting screenshots were inspected. The complete
Swift suite passed with rendering enabled: 117 UI tests and 14 core tests (131
total), including the previously intermittent process-transport tests. Resource
consistency and diff whitespace checks passed. Native menu and preference
interaction are checked separately in the packaged QA app.

WinUI 3 continues to consume the unchanged typed artifact/file records. Its view
should implement the same shared, client-local list/grid preference and route the
artifact Provenance action to the existing provenance panel; neither operation
requires a new backend command. File editing, remote operations and downloads
remain on the full parity checklist.

Packaged QA interaction passed: switching Artifacts from list to grid updates the
selected accessibility value; Files inherits grid mode; opening README.md still
loads its real synthetic contents and immediate Escape closes the preview.
Closing and reopening the side panel preserves grid mode. The artifact context
menu's immediate Escape closes only the menu, leaving the panel visible. Selecting
Provenance from that menu opens its previously absent tab and displays the recorded
Python step. The rebuilt QA bundle passed strict signing verification.

## Tool-input selections

Expanded tool inputs now use the same native selectable text component as tool
outputs: contextual quotation, confirmed excerpt saving, saved-excerpt underlines
and the configured code font. Revealing a saved excerpt searches each tool's input
and output separately, expands the matching tool, and marks the selected range.
It does not join the fields (which could manufacture a match across their boundary)
or search a non-tool item's hidden input field. Regression coverage checks input,
output, cross-boundary rejection and the non-tool case.

The shared highlight save/read contract is unchanged. WinUI implementations should
apply `NativeSavedExcerpt.Find`/`FindAll` separately to `ConversationItem.Input` and
`Text` for tool rows, render input as literal code, and expand the matching row when
navigating from Highlights. The existing scoped `StarAsync` method accepts either
selection without a new command or platform-specific DTO.

The complete Swift suite passed after this change: 118 UI plus 14 core tests
(132 total), with offline rendering enabled. The QA package rebuilt and passed
strict signing verification. In the actual synthetic conversation, selecting the
expanded Python input exposed both contextual actions. Saving it added the
underline after the backend confirmed the saved highlight; quoting it opened
SideChat and displayed the exact code as a conversation excerpt without sending a
question. No provider request was made for that quotation check.

## Share Markdown structure

The native share preview and PNG renderer now preserve pipe tables (including
column alignment, escaped pipes and inline-code pipes), ordered-list numbering,
indented list items and heading levels. Fenced code keeps the opening fence's
actual length and requires a matching closing line; a three-backtick example no
longer terminates a four-backtick outer block. Table rows with omitted trailing
cells are padded, while a non-divider pipe line remains a paragraph.

Regression tests cover these parsing cases and render the shared
`share-complex.json` fixture at 320 and 840 pixels through the actual PNG exporter.
Both PNGs were inspected: the table is legible, list nesting is visible, and code
is rendered literally without cropping in this fixture. The full Swift suite
passed (121 UI + 14 core = 135); after moving the sample into the shared fixture,
the seven share tests and full C# contract executable passed again. WinUI's
contract test verifies that table and list Markdown survive decoding unchanged;
it does not claim a WinUI renderer exists.

This closes specific structural gaps, not the full Markdown parity item.
Multi-paragraph list items, nested block quotes and broader WebView comparison
remain to be handled alongside the other original acceptance requirements.

## Terminal callback identity

The terminal emulator coordinator now retains the terminal ID from the view's
creation. Its input and resize callbacks pass that identity through the main-actor
hop; the model ignores a callback if a different terminal is selected by the time
it arrives. Previously those callbacks looked up the selection only after the hop,
which could redirect an old view's queued keystroke or size event to a new terminal.
Already accepted operations still retain their original terminal ID and ordering.

Two regression tests cover both the model boundary and the actual SwiftTerm
coordinator's delayed callbacks, plus valid current-terminal input and resize.
WinUI's existing terminal methods already take an explicit terminal ID; its eventual
view coordinator must likewise capture the originating terminal instead of reading
the current selection later.

Inspection of the pinned SwiftTerm implementation confirmed that ordinary Copy
and Paste use its built-in responder actions; the empty clipboard delegate methods
are OSC 52 programmatic access, not the user's keyboard shortcuts. They remain
unchanged. Live resize, clipboard and high-volume rollover checks remain on the
terminal acceptance list.

Full Swift verification after the callback fix passed with offline rendering
available: 123 UI and 14 core tests (137 total). No backend or DTO shape changed.

## Execution-context terminal shortcut

Attached execution-context cards now expose a Terminal action beside Runtime and
Runs. The workspace routes that action's context ID to the existing scoped terminal
open command and makes the terminal panel visible. The terminal model is retained
per database/project/session, so the card action and bottom panel operate on the
same selection; another project's or session's panel gets a different model.
Explicit launches suppress the panel's default local-terminal creation, including
while the requested open is pending or has failed. Concurrent duplicate clicks are
coalesced while an open is pending, and uncertain opens are reconciled without
replay.

Regression tests cover explicit remote-context forwarding without a real SSH
connection, duplicate suppression, no automatic fallback after failure and scoped
model reuse. The full Swift suite passed: 125 UI and 14 core tests (139 total).
WinUI can wire its context card to the existing typed terminal open method with the
card's context ID; no new backend DTO or command is required.

The rebuilt, strictly signed QA app passed live checks in `toolbar-qa`: clicking
Local's Terminal action showed a working terminal in the synthetic workspace.
`stty size` reported 15 rows by 59 columns; dragging the splitter to increase its
height changed the result to 23 by 59. Pasting a `printf` command through the native
paste action produced `NATIVE_PASTE_OK`. These checks establish local launch,
vertical resize propagation and paste; remote execution and high-volume cursor
rollover remain separate acceptance items.

## Packaged terminal scrollback rollover

Live QA now covers output exceeding the backend's 4 MiB retention limit. In the
synthetic `toolbar-qa` terminal, a delayed Python command emitted 70,000 ASCII
lines followed by `NATIVE_ROLLOVER_END` while the panel was hidden. After reopening,
SwiftTerm displayed the final marker and shell prompt. A new `printf` then produced
`AFTER_ROLLOVER_OK`, establishing that input and incremental output still worked.

An authenticated read against the same isolated host (after validating its
canonical database identity) confirmed actual rollover rather than merely a large
render: `start=1551747`, `end=5746051`, `retained_bytes=4194304`, `reset=true`, and the
end marker present. No credentials were printed. The probe was read-only and used
the existing scoped terminal API. This satisfies the local large-output/reconnect
smoke case; it does not establish remote SSH/WSL behavior or all terminal emulation
sequences. Earlier notes that local rollover was unverified are superseded.

## Editable local text previews (implementation in verification)

Workspace text files opened from Files now expose Edit and Save. The editor keeps
an unsaved draft, asks before discarding it, and blocks dismissal while saving.
Artifact previews and partially loaded/binary previews do not expose editing.
The native save command resolves the explicit frame's workspace, rejects archived
sessions or non-writable scopes, validates the existing file boundary, compares
its current full text to the editor's original text, and uses the existing bounded
file-save implementation. A differing baseline or truncated read rejects the save.
State generation is bumped after the write.

The new `native_conversation_panel_savefile` request carries `session_id`, `path`,
`original_text` and replacement `text`; its confirmed result is `true`. The command
is in the shared Rust allowlist, and C# `INativePanelClient.SaveFileAsync` exposes
this same contract without retrying mutations. Swift updates its preview only
following a confirmed result; failed/uncertain saves retain the draft and show an
error. This addresses local text editing; file creation, remote editing and
artifact snapshot mutation are separate behaviors.

Current checks: complete Swift suite passed (127 UI + 14 core = 141), including
baseline forwarding, uncertain-save state, artifact rejection and truncation
rejection. C# contract checks passed for scoped payloads and no replay on lost
responses. The new Rust test exercises successful replacement, changed baselines,
missing files, parent-directory escape and oversized previews. Rust verification
and packaged editor interaction are still in progress; this feature is not yet
counted as fully accepted.

## Runs and Memos tabs (SwiftUI)

The SwiftUI right panel now has the WebView's 运行 and 备忘录 tabs.

运行 lists this conversation's runs and file transfers (`frame_id` equals the
session; runs without a frame are left out), split into 进行中 and a collapsed
已完成 group with the same ordering as `session_run_groups`. The tab title counts
only active runs. Rows reuse the existing activity model and commands for detail,
cancel (confirmed first) and re-harvest; the tab polls every 2 s while shown.

备忘录 lists the project's private memos, pinned first then newest, with search,
new, edit, pin/unpin, copy, delete and 插入输入框. Insert appends the body to the
main composer draft after a blank line, matching `compose_insert`; nothing is
sent. The four memo commands are on the native settings allowlist and run in the
project's hidden host context; a memo returned for another project is rejected.
Escape closes the inline editor first (asking before discarding changes) and
leaves the panel open. `contracts/native-settings/v1/project-memos.json` is
decoded by both wisp-dto and Swift tests.

The WinUI `NativePanelTabs` registry does not include these two IDs yet.
