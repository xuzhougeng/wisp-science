# Windows native parity: PRs #1332–#1351

The WinUI preview now connects the native workbench actions introduced in this
range to the existing Rust desktop host. No second database writer or WebView
project activation was added. Build with `scripts/build_native_windows.ps1`.
An alternate browser database still requires a matching running host descriptor.

## Interface improvement work in progress 2026-10-02

The [visual improvement plan](superpowers/plans/2026-10-02-winui-webview-visual-improvement-plan.md)
now has an initial implementation in the working tree:

- Conversation and composer use the same 1280 DIP maximum. The workspace budgets
  at least 560 DIPs for the center when the client width permits. Inspectors
  become overlay drawers when docking would squeeze the center; very narrow
  windows can open the automatically hidden sidebar as a drawer.
- The inspector has a draggable, keyboard-adjustable width, saved locally.
  Its selected tab has an explicit toggle state and is saved. Overlay drawers
  participate in the window Escape order, below sheets and flyouts.
- Inspector tabs retain their own filter and scroll position for the mounted
  session; file directories retain separate reading positions. Changing a filter
  returns to its first result. Late reads from a previous tab, directory or
  closed panel cannot replace accepted data. Scroll restoration after layout
  still requires actual WinUI acceptance, beyond the passing state-model tests.
- Settings navigation searches the shared Chinese/English labels, groups and
  aliases without switching the current form or discarding its draft.
- Appearance and category pages share a 1000 DIP content limit and 28 DIP
  inset. Editor save/cancel actions remain in a fixed footer, stacking in narrow
  columns, with scrollable feedback. General and network forms have task-based
  sections. A confirmed save remains confirmed if the following refresh fails;
  a new edit resets that feedback. Failed writes continue to retain drafts.
- The composer groups attachments, model, environment and primary actions in a
  compact toolbar which wraps in narrow columns. A leading slash filters wired
  commands; arrows select, Enter/Tab fills the command, and Ctrl+Enter executes.
  Escape dismisses suggestions through the window stack. IME composition does
  not submit. Unknown commands and extra arguments retain the draft, and upload
  commands clear only after a confirmed attachment in the same conversation.
- Built-in conversations expose a Plan toggle when the host advertises the
  optional `plan_mode` snapshot field. It uses the existing session Plan tool
  gate and persisted flag; ACP agents retain their own mode mechanism. Running,
  read-only, historical and disconnected conversations cannot switch modes.
  A pending write blocks sends; polling never writes, and failed responses are
  not replayed. Old hosts omit the control. Actual native Plan submission and
  keyboard/narrow-layout acceptance remain pending.
- Fast is available for the same built-in OpenAI chat transports as the WebView
  composer. It requests the provider's priority service tier and identifies
  whether the current value is inherited or overridden for this conversation.
  Switching to the model default clears the override; explicitly turning Fast
  off overrides a priority default with the ordinary service tier. The command
  checks the displayed model ID and rejects unsupported, ACP and running
  sessions. Old hosts hide the control. Saves block competing sends and are not
  replayed after an uncertain response. Provider billing and availability still
  apply; actual native end-to-end request acceptance remains unverified.
- Cancelling a pending conversation action releases the busy state without
  assuming the write was cancelled on the host. Sending stays blocked until a
  fresh snapshot confirms the current state; no write is automatically replayed.
- The environments panel displays the conversation's saved default separately
  from attached environments. An unset legacy value inherits global settings;
  choosing local pins this machine, and choosing a remote context uses the
  existing host operation that also enables it for the conversation. It does
  not alter the global default or other conversations. Older hosts hide these
  actions. Uncertain writes block further environment changes until a read-only
  refresh confirms state; refreshing after a successful change releases Busy.
  Reads started before or during a context write cannot resolve its uncertain
  result. Only a fresh post-write read enables further changes. Completing a
  write after switching tabs keeps the selected tab and waits for fresh data.
  Actual remote setup, first-server storage preferences and native end-to-end
  execution acceptance remain pending.
- Unchanged transcript controls survive polling. Row keys use the persisted
  user-turn offset, session and host epoch, with page isolation for older hosts.
  Updated rows preserve disclosure state; reading away from the bottom exposes
  a return-to-latest action. Unchanged approval controls also stay mounted.
  Historical paging and outline navigation share a request generation: switching
  conversations, returning to latest, or choosing a newer history destination
  discards late pages and errors from the abandoned request.
- Completed process phases share a show/hide control while the final report
  and trailing usage stay visible. Toggling preserves mounted message controls
  and nested tool disclosure states. Failed, pending, unknown and actionable
  rows split groups; active turns remain visible. Run monitors remain separate
  until the native protocol can establish their ownership and completion.
- Tool disclosures show explicit completion, running, cancellation, failure or
  unknown state plus the host's recorded elapsed time when present. Output is a
  two-line wrapping preview rather than a fixed UTF-16 substring. Existing
  optional duration/model/timestamp metadata now survives native decoding; old
  hosts do not acquire fabricated timing. Explicit error status expands a tool
  even when `ok` is absent and its previous successful view was collapsed.
- The terminal uses bundled xterm.js in a WebView2 surface with local-only
  navigation, no host objects or external resource fetching, and a per-user
  WebView2 data directory. It supports terminal selection, VT output, Ctrl+C,
  resize messages and draggable/keyboard-adjustable height. Empty lists do not
  start a process automatically; use New Local Terminal or the context action.
  Hiding keeps the process alive. Ambiguous input blocks further input on that
  terminal until the user checks output and resumes; queued input is never
  replayed into another terminal. If the view fails, output is shown read-only.
- Inline and display math use Markdig math nodes and bundled KaTeX in a
  read-only WebView2 renderer. Invalid formulas retain their source; trusted
  HTML and external formula links are disabled. File/artifact PDFs returned by
  the existing host render with bundled PDF.js, page controls and zoom. This
  first canvas viewer does not provide text selection, search or annotations.
  Truncated/missing PDF bytes and rendering errors have explicit feedback.
  Unmounting closes browser resources. Dismissed or superseded preview reads
  cannot reopen or replace the current preview.
- Research Journey groups records by local date and filters type, title and
  summary. Details show metadata and immutable artifact versions, their input
  sources and partial content errors. Source chains have a back path; source
  conversations open as nested read-only pages so filters and parent controls
  remain mounted. Returning to the list restores its position and highlights
  the selected record. Refresh failures retain and identify previous results.
  Publication evidence has a revision summary, counts, grouped items and
  distinct empty states; editing/binding/reproduction remain separate work.
- Home and sidebar share session status labels and relative timestamps with
  exact-time tooltips. Running sessions are no longer mislabeled as complete;
  unknown statuses remain unknown. Project cards include running/attention
  counts and update time. A project overflow button exposes settings and folder
  reveal alongside the existing context menu. Initial loading and failed reads
  are distinguished from an empty project/session list.
- The composer exposes model-default thinking effort for exact catalog entries
  with supported values, matching the current WebView persistence behavior.
  Saving reads the latest profile, preserves unrelated fields and sends no key.
  Only a matching host acknowledgement updates the displayed default. Unknown
  catalog entries expose no guessed controls. Read-only/history/running states
  disable the picker; errors offer read-only recovery, never automatic replay.
  This does not implement Plan, Fast or session execution-context defaults.

Validation so far: WinUI Release build and the full C# contract harness pass,
including column-budget, navigation-search, transcript-identity and completed
process boundary tests. For manual acceptance, open a finished multi-tool report,
toggle its process, and check the report and usage stay visible; expand a tool,
wait through polling, and verify both disclosure states persist. Repeat with a
failure, pending question and an active turn, which must remain visible.
These tests do not prove XAML rendering, keyboard focus, text selection or DPI
behavior. Current Windows Computer Use initialization fails with a sandbox
helper startup error, so real-window acceptance and normalized screenshots
remain pending. Settings smoke checks must cover long network forms at large
font sizes, save failure with retained values, and switching categories then
cancelling the discard prompt. Run-monitor grouping, message action styling,
additional rich viewers, protocol additions and full visual acceptance remain open.

Home/sidebar presentation tests cover known/unknown status values, time
boundaries, future clocks and invalid timestamps. Manual checks still need
long Chinese/English project titles and paths, a narrow window at large font
size, keyboard access to overflow menus and immediate Escape dismissal.

Journey model tests cover explicit timezone date boundaries, combined filters,
failed refresh with retained results, immutable source identities, input-source
back navigation, missing sources and late replies after return/disposal. Manual
acceptance still needs calendar → journey → source conversation → back, retained
filters/scroll, immediate Escape, missing source content and partial-range data.

Offline rich-preview Playwright coverage loads actual KaTeX/PDF.js resources:
valid and invalid formulas, inert external links, PDF page pixel colors, page
navigation, zoom and corrupt-file errors. The C# harness checks dismissed and
out-of-order preview responses. Real-window acceptance must still cover inline
formula baselines, long formulas, theme/font/DPI changes, PDF sizing in narrow
panels, repeated close/reopen and WebView2 failure. Office, molecular, MSA and
MCP App viewers are still pending evaluation.

The native terminal's offline Playwright test loads the actual bundled xterm
modules and verifies ANSI/CR rendering, Chinese text, Ctrl+C, disabled input,
resize messages and stale-stream isolation. C# tests cover split UTF-8, ordered
writes, per-terminal uncertainty, late responses and detach. These do not prove
WebView2/PTY integration: manually test terminal selection, local/context open,
shell editing, fullscreen terminal programs, window/DPI resizing, hiding and
reopening, process exit and a disconnected host on the actual Windows build.

## Reading, panels and session actions (2026-10-01)

Gap audit and batches W1–W5 from
[the WinUI parity plan](superpowers/plans/2026-10-01-winui-webview-parity-gap-plan.md)
landed in the WinUI preview against the shared host, with no new host commands:

- Reading: pipe tables render as real grids, code fences get token
  highlighting from `NativeCodeHighlight` (unknown languages stay plain), a
  language label and a copy action; per-turn usage rows render as the compact
  `输入 x · 输出 y tokens · 缓存 z · 思考 w` line; local markdown images and
  `view_image` tool results render inline from existing file paths. Formulas
  still render as source; remote images are never fetched.
- Panels: artifacts group by type with counts, image artifacts/files preview
  from host base64, notebook cells show collapsible output, the agents tab
  gains the explanation and delegation-state cards (delegation toggle with
  no-retry semantics), and the tab strip scrolls horizontally.
- Session actions: per-session menus wire `native_conversation_rename/pin/delete`
  (confirmation for delete, failures never retried), message rows gain 复制,
  and `BrowserSession` decodes the optional `pinned` field so a leading
  已置顶 section appears in every grouping mode.
- Runtime console: the hosts tab lists live runtimes (stop/restart/dismiss),
  run records (detail, cancel, harvest) and an execute panel (context +
  python/R + code) over `NativeContextActivityClient`; reads never start a
  runtime and all mutations are guarded, ambiguous ones reported without retry.
- Composer: an 环境 entry opens the hosts panel and a client-side slash
  subset (`/upload /files /outline /share /trajectory /archive /library
  /calendar /journey /publication /settings /scratch`) routes to existing
  native surfaces; unknown commands keep the draft and list the set.

Automated validation: the complete C# contract harness passed, including new
`NativeTranscriptTests` (usage formatting, tokenizer, image paths) and
`NativeWorkspaceActionsTests` (artifact grouping, pinned sections,
rename/pin/delete semantics). Release publish and a live preview walkthrough
(same project/session on both surfaces) verified tables, usage lines, message
actions, the slash hint, session menus and the agents/delegation cards. W6
decisions (WebView2-based VT terminal and rich viewers; deferred items) are
recorded in the plan.

| Reference | Windows behavior |
| --- | --- |
| #1332–#1333 | Native search already scrolls its selected result into view. Home now opens the same tutorials URL. The WebView command palette and Help menu fixes remain shared. |
| #1334, #1336 | Native create/import forms, Windows folder/ZIP pickers, editable project context, optional standard layout, and navigation after the authoritative project summary. Failed writes preserve fields and are not replayed. |
| #1335 | Sidebar Files reveals and selects the files tab for the open native session, including when that tab was previously closed in the saved layout. |
| #1338, #1348 | Name/recent sorting, folder/date/no grouping, selection, moving sessions, creating and renaming groups. A partial move retains unresolved selections without replaying confirmed successes. |
| #1339 | Shared library search and kind filters, deletion, source navigation, and insertion into an open composer without sending. |
| #1340, #1348, #1350–#1351 | Local month/day calendar, project filter, partial errors and truncation, dated journey, and host privacy gate. Hidden project IDs never enter the calendar payload. Pending/failed privacy reads disable navigation; no fallback or automatic retry. |
| #1341 | Project-scoped journey with editable date range and local title search. Calendar journeys are nested above the calendar so Escape restores the chosen day. |
| #1342 | Publication workspace in the project column, initial revision creation, retained draft on failure, and Escape back to the conversation. |
| #1343 | Capability summary now opens the corresponding same-window settings category. Skills support local import, enablement, tags and file inspection; connections support connector/tool approval controls and MCP editing; memory supports project files, global entries and failure-analysis preferences. The remaining settings and acceptance work is tracked below. |
| #1344 | Feedback prefills the current composer with version, platform, model and startup information. It neither sends nor includes the workspace path; stale reads and intervening edits are preserved. |
| #1345 | Independent hidden scratch conversation using the same native conversation loop. Close/Escape names only the scratch project; failures are not automatically retried. Exiting the process leaves orphan cleanup to the host's existing startup purge. |
| #1346–#1347 | Local attachments are staged per session, included in send/enqueue payloads and shown on saved messages. One follow-up is allowed while a turn runs. An uncertain queue preserves the draft and requires explicit user acknowledgement before another submission. |
| #1337, #1349 | These are WebView fixes, already present on the shared baseline. Opening folders without a native session is not a new WinUI capability in this increment. |

## Lifecycle and navigation

Native surfaces stay in the existing window. The window Escape stack closes
flyouts before child sheets, child sheets before their parents, and publication
content before returning to the conversation. Native pickers retain their own
Escape handling. Closing a read surface invalidates outstanding replies. A
project/database change resets project-scoped models and the host connection
cannot be reused for another database.

Reusable WinUI conversation/panel controls are detached through their retained
container before rebuilding the surrounding layout. `FrameworkElement.Parent`
can be null after unloading even while the old container still owns the control;
relying on that value caused a blank project page with COM error `0x800F1000`.
Rendering also defers reentrant notifications and provides a recovery action
instead of leaving an empty window.

## Settings and typography (#1357)

All 20 settings categories open in the existing WinUI window. Native editors
cover general/network/session/appearance/pet preferences, skills and SkillStore,
connections/MCP, memory, plugins/browser, credentials, permissions,
environments, storage/usage, API/ACP models, workflows/actions/specialists,
channels and project settings.

Settings models retain unknown fields and credential references, preserve
failed drafts, reject duplicate saves, and ignore late replies after disposal.
Secret writes use existing host keyring arguments. Model editors submit all
image/video assignments explicitly. Authentication terminal input is scoped,
not persisted or automatically replayed after uncertain delivery.

Project scope changes and category navigation respect unsaved-change guards.
Discard confirmation and OAuth cancellation stay above long scrollable forms.
Project settings preserve identity, save Agent Context through the host and
refresh home/recent-session summaries after closing.

Workflow editors preserve task graphs, nullable capability inheritance and
conversion provenance. Built-in workflows remain read-only and can be copied.
Channels include project sync, Feishu/Lark, Weixin and device bridge controls.
Binding polls are explicit, honor retry intervals and retain failed cancellations.

Committed UI/code font preferences bind to conversation prose, Markdown,
composer, settings navigation/forms, shared sheets, search, auxiliary panels
and terminals. The traversal visits app-owned logical content, preserving
renderer fonts and existing bindings; it avoids generated templates and icons.
Unsaved appearance changes remain local to the preview. Defaults are 14/12,
bounded UI/code ranges are 12-20 and 10-20, and heading proportions are retained.
Controls inserted after initial rendering bind immediately, including added
workflow tasks and SkillStore preview fields. Disclosure headers use explicit
bound text and preserve their accessible names; WinUI's default header template
otherwise kept those labels at the default size. The obsolete appearance-page
notice that other categories were not yet available has been removed.

Settings navigation restores the themed button background after deselection.
Assigning a null background made only the text label hit-testable, so clicks in
the center/right padding stopped switching categories after the first switch.
The full navigation row now remains clickable.

Publication renders the host-selected paper with its matching revision/items.
Initial creation is offered only after a confirmed empty workspace read and is
replaced by the confirmed paper. Audited PR #1342 explicitly excludes evidence
binding, readiness and reproduction; those are not extra parity requirements.

## Automated validation (2026-09-24)

Passed locally:

- WinUI Release build/publish and the complete C# contract harness, including
  35 initial parity checks, 29 settings-editor checks, 16 model/authentication
  checks, 13 channel checks, six typography checks and publication lifecycle
  coverage. These test scope, draft retention, duplicate guards, null/empty
  inheritance, casing, failed cancellation, late replies and no automatic replay.
- Rust native settings DTO tests, wasm UI check, native design/settings contract
  synchronization, Rust formatting and diff checks.
- All 989 Tauri unit tests after correcting the Windows test manifest. Following
  final resource unification, a fresh relink passed both publication tests;
  the production app build and extracted manifests also passed verification.
- The SSH ledger test and deterministic progress/upload interleaving regression
  after the fixture correction described below.

The full `cargo test --workspace -- --test-threads=1` run finished with exit code
0, including all 174 `wisp-runs` tests, Tauri tests and workspace doc-tests.
The final dynamic-control font build/publish and complete C# harness also passed.
The final full four-worker Playwright run passed: 869 passes, two skips, exit
code 0 (24.8 minutes). It used the same built frontend assets served over
HTTP/1.1 keep-alive. An earlier full run had 864 passes, two skips and five
failures; all five failure locations also passed a focused six-case rerun.
The earlier failures and test-server distinction remain recorded below.

### Regression findings and fixes

The Tauri test executable lacked the Common Controls v6 manifest present in the
app and exited with `0xc0000139` before assertions. The MSVC linker now embeds
one shared manifest across executable targets; the app-only manifest resource
is disabled to avoid duplicate-resource CVT1100. Other target configurations
retain the default Tauri behavior. No binary-patching workaround is required.

The original workspace run had two `wisp-runs` failures; focused reruns passed,
then repeated package/workspace runs exposed intermittent failures again. A
later full diagnostic package run passed 173 tests, which alone did not resolve
the flake. Command diagnostics subsequently proved the ledger fixture race:
`prepare SSH Run`, `poll SSH input progress`, `stage 1 input file(s)`. Progress
consumed the scripted upload-success response, so upload received the launch
failure and correctly wrote no ledger. The fake runner now answers progress
queries independently of the mutation-response queue, with a deterministic
interleaving regression. The four-thread workspace run separately reported
SQLite `database is locked` in the harvest lease test; that remains unresolved.
A subsequent four-thread `wisp-runs` package run passed all 174 tests and
doc-tests. That pass does not establish that the earlier SQLite flake is fixed.

The first fresh Playwright attempt failed at server startup before tests ran.
A separate Trunk build succeeded and the completed rerun served those assets
from an isolated static server. Failures were the cold-window loading indicator,
three project-entry timeouts (message evidence, overflow tabs and Chinese
project export), and the two-second Stop action in the WebView load test.
Generated research-journey design-QA PNGs were restored after the suite finished.

Hydration regressions now hold the next snapshot with an explicit promise gate,
so loading-state assertions and intervening live events complete before the
snapshot is released. They no longer depend on 350/400 ms timing windows.
The first repeated run had 15 passes and one listener-readiness setup timeout.
Serving the identical frontend assets with HTTP/1.1 keep-alive and a larger
connection backlog then passed all 16 cases (three workers, two repetitions).
This suggests a test-server contribution; it does not establish the cause of
every original timeout. The subsequent full four-worker run passed 869 tests
with two skips and exit code 0. Generated research-journey design-QA PNGs were
restored again after this final run finished.

Historical #1355 validation: Playwright had 867 passes, two skips and one
queued-guidance timeout; all seven focused queued-guidance tests then passed.
Its original Rust run stopped at the now-corrected Tauri loader failure. Those
historical results do not substitute for final #1357 regression results.

## Real Windows and host acceptance

The production Rust host was built with isolated identifier
`science.wisp-science.parity-acceptance` and its own application database.
Acceptance used disposable projects, not the user's research data. The native
broker forwarded real Tauri invokes. Session persistence checks read the
project's `.wisp/project.sqlite`, not the application registry database.

Verified against that host:

- Project creation, valid ZIP import with identity and exact extracted bytes,
  publication create/read, group creation/rename and session group assignment.
- Project name/Agent Context save/read, scratch open/close, conversation
  creation and exact-byte attachment copying into the project workspace.
- Real Rust agent execution with a disposable loopback HTTP model fixture:
  attachment-bearing first turn, exactly one queued follow-up and two assistant
  replies. Restart preserved both turns and the attachment transcript.
- Workflow graph save/read, quick-action template binding and specialist
  instructions with inherited skills (`null`) versus an empty connector list.
  All survived host restart together with the earlier project/publication data.
- Conversion from a disposable project Skill through a loopback planning model:
  source read, two-node draft validation, explicit template save, graph readback
  and persisted source SHA-256. This verifies the host contract and provenance;
  it is not external model-quality acceptance or workflow execution.
- A uniquely named ZIP skill through the real install command: exact copied
  files, reference listing/readback, tags, project enable/disable, reload and
  removal. The temporary global package was removed without replacing existing
  skills. This does not establish pinned GitHub download/install acceptance.

Actual WinUI acceptance:

- Navigation hit-testing regression: reproduced the old failure with center
  clicks on Appearance/General after opening Workflows; clicking the label
  still worked. After restoring the themed background, center/right-padding
  clicks switched Workflows → Appearance → General, and General → Appearance
  at a captured 616x567 window. Release publish and the complete C# contract
  harness passed. The manual regression procedure is in `native-settings.md`;
  model tests alone do not verify the native template's hit-test behavior.
- Synthetic fixture host: memory edit/save; same-window general/model/ACP forms;
  quick-action Escape; project-scoped workflow save with graph preservation;
  selected-project rename and refreshed summaries; topmost discard Escape.
- Real host: imported/created projects and matching publication/revision display;
  initial-create form absent for an existing publication; immediate Escape
  returns to the project area with sidebar retained.
- Recovery buttons are absent on ordinary empty conversations, appearing only
  for an error or uncertain send/queue result.
- UI/code font sizes 20/18 enlarge home/settings text and controls. Restoring
  14/12 and reloading the same mounted page updates existing controls immediately.
  Original isolated-host preferences were restored.
- With UI size 20, a task added after opening the workflow editor displayed
  enlarged input fields. The final build also rendered enlarged disclosure
  labels with the expected accessibility names. Restoring 14/12 and reopening
  settings confirmed the original preferences were active again.
- Dirty project-scope switching opens the visible top confirmation while
  retaining the original project. Immediate Escape dismisses only confirmation,
  retains the draft and re-enables the editor. Explicit discard restores the
  saved project name.
- Real remote-access overview and unbound Feishu/Lark detail load correctly;
  Escape returns to the overview. At a verified 614x567 captured window, sidebar,
  overview/detail controls and wrapped explanatory text remained usable, and
  detail Escape retained the settings parent. No account binding was started.

## Remaining acceptance

The overall parity objective remains open. Investigate the earlier intermittent
parallel SQLite failure; final serial workspace and four-thread package runs
pass, and the final full Playwright run is green. Further manual acceptance covers
150% display scaling, additional narrow/large-font dynamic surfaces, real
provider/ACP authentication, MCP test/OAuth cancellation, pinned SkillStore
installation, channel binding/sync and real SSH/WSL contexts. External-account
flows must use dedicated test accounts; loopback fixtures do not prove those
integrations work. Preserve the distinction between automated model/transport
checks, actual WinUI checks and real external-service acceptance.

## Conversation secondary actions (2026-10-02)

Copy, quote and save remain visible native text buttons below message content.
They now use compact spacing, transparent backgrounds and muted text; pointer
hover or keyboard focus emphasizes the text without hiding actions or changing
Tab order. No menu or extra Escape layer is introduced. These actions remain
available while a later turn is running; the existing action handlers are reused.

Release compilation and the full C# contract runner pass. These checks do not
exercise WinUI pointer/focus rendering. Manual regression steps remain pending:

1. Open a conversation with user messages and a final report. Confirm all three
   actions remain visible without hover, and the final report dominates visually.
2. Tab to each action and activate it with the keyboard. Confirm a visible native
   focus indicator, correct copy text, quoted draft and saved selection.
3. Hover an action, focus it, then move the pointer away. It must remain emphasized
   while focused; moving focus away must restore its secondary text color.
4. Start a later turn, then copy and quote an earlier message. Polling must not
   remove focus from an unchanged message or disable read-only history actions.
5. Repeat in light/dark and Windows high-contrast themes, at large font sizes and
   150%/200% display scale. Check contrast, unclipped labels and usable hit targets.
6. With touch, activate the always-visible controls without requiring a hover step.

## Transcript Run ownership and folding (2026-10-02)

Conversation snapshots now optionally attach stored Run state to exact submission
and monitoring tool rows. The host resolves the requested conversation's project
and exploration scope, then matches the Run ID and owning frame. It never guesses
ownership from titles, commands or timestamps. `owner_index` is local to the
returned page; a submission outside that page leaves ownership absent.

Successful monitors with a successful submission in the same page can join the
completed process disclosure, including their preceding commentary. Active,
failed, cancelled, unknown and unowned monitors stay outside it. SSH-direct Runs
that still expose results review remain visible, and their details expand. The
final answer stays outside the process group. Monitoring tool success is no longer
presented as proof that the Run finished; tool elapsed time is explicitly labelled.
Older hosts without this metadata display an unverified Run state.

The shared `transcript-runs.json` fixture verifies Rust/C# decoding and successful
folding. Model regressions cover stale/wrong/missing ownership, active turns,
failures, pending review, unknown states and old hosts. Host projection tests cover
live stored state, malformed submission results and foreign frames. These do not
replace the following pending real-host checks:

1. Submit a local fixture Run and monitor it. Verify the header changes from
   running to completed when stored Run state changes; the final answer remains
   visible when the completed process folds.
2. Force a nonzero exit and a timeout. Verify the relevant status stays visible
   and the tool details expand, even when the monitor call itself succeeded.
3. Page away from the submission and switch conversations during a refresh. No
   unrelated Run may acquire an owner or fold into another conversation's process.
4. Verify a completed SSH-direct Run with results review available remains visible.

Live inline Run cards, direct review navigation and full WinUI window acceptance
remain follow-up work. Exact detail navigation is now available as described below.

## Open Run details from the transcript (2026-10-02)

Submission and monitoring rows with verified Run metadata expose “查看运行详情”.
The action opens the current conversation's environment panel and places the
selected Run at the top, showing its state, exit code, working directory, command,
stdout/stderr tails and poll/cleanup errors. “刷新详情” explicitly reads another
snapshot; these output tails are not a live stream. Opening details performs reads
only. It does not start a runtime, cancel a Run, harvest outputs or dismiss review.

Choosing a different Run clears the previous Run's output immediately. Separate
request identities reject out-of-order reads; tab changes, inline dismissal and
panel disposal invalidate pending requests. A failed refresh of the same Run
keeps the previous snapshot with a visible stale-data notice. Returned Run IDs
must match the requested ID. Window navigation guards also abandon pending opens
when the user changes conversations, panels or settings.

Run cancellation/harvest controls are hidden in read-only activity panels. Only
known active states offer cancellation and known terminal states offer harvesting.
An activity operation now releases its busy flag after its own panel refresh;
cancelled or mismatched responses report an unconfirmed outcome without replay.

Automated coverage lives in `NativeRunNavigationTests.cs`. Pending real-window
acceptance: open a Run from a long transcript with the side panel initially closed;
confirm its details are visible at the top; select two Runs rapidly; close the
detail or panel while reading; switch sessions and settings during host connection;
verify a failed refresh is labelled and no read starts a runtime. In a narrow
window, immediate Escape must close the existing topmost panel drawer only.
Manual results review and output selection/cleanup are now implemented below.
Live inline cards and automatic review prompting remain open.

## Native Run results review (2026-10-02)

For finished SSH-direct Runs, “结果与清理” opens a review inside the existing
environment panel. Hosts explicitly advertise `run_review_supported`; older
hosts do not show a nonfunctional review action. The protocol resolves the named
session's project/exploration scope, allows inherited Runs to be browsed, and
requires the owning writable scope and an unarchived session for changes.

Review supports directory navigation, name filtering, pages of 200 entries,
selection across directories, and download of selected files or archived
directories into registered project artifacts. Selection uses exact relative
paths. Deleting a selection displays the frozen path list before confirmation;
whole-workspace cleanup separately warns that unretained contents will be lost.
Both actions require an explicit confirmed operation in the typed host request.
Existing RunManager checks and log preservation are reused. No content is
downloaded or deleted merely by opening the review.

The window Escape stack closes confirmation first, keeping review and selection,
then returns to Run details, then closes a narrow-window panel drawer. Returning
from a settled review refreshes the Run detail. Closing a pending operation does
not replay or undo it. Unconfirmed responses retain selection, disable further
writes until a fresh read, and show an explicit error. Read-only scopes cannot
download, delete or clean. Scope changes, late responses and closed pages cannot
replace another review's state. Successful acknowledgements remain visible even
if the following listing refresh fails.

Automated verification uses the shared `run-review.json` fixture and
`NativeRunReviewTests.cs`, plus existing fake-runner tests in `wisp-runs` for
selected downloads, cleanup boundaries and full-log preservation. Tests use no
real SSH server or external data. Pending real-window acceptance: open review
from details, browse/filter/page/select with keyboard and pointer, inspect all
confirmed paths, immediately press Escape at both levels, use a read-only scope,
and switch sessions during listing and mutation responses. Any real cleanup
acceptance must use a disposable fixture workspace, not a research directory.
Automatic end-of-turn prompting and persisted prompt dismissal are not added by
this manual-review slice.

## 研究历程运行来源补充（2026-10-02）

历程中的运行记录及产物生成运行提供“查看来源运行”。通过项目主线作用域和精确 Run ID 读取已保存的状态、命令、远端工作目录、退出码及 stdout/stderr 尾部，不切换当前会话，也不启动运行时或触发远端轮询。页面只读；返回或 Escape 保留父级产物来源、筛选和滚动位置。跨项目、缺失运行与身份不匹配返回错误；关闭或返回后的晚到响应被丢弃。旧宿主不支持时显示读取错误，不提供虚假结果。

验证：新增 C# 契约与模型测试覆盖精确身份、返回状态、晚到响应及不匹配拒绝；宿主测试覆盖项目隔离、缺失身份和拒绝写入参数。实窗 smoke 尚待完成：历程→运行→返回、产物→输入来源→生成运行→Escape，以及读取期间返回和窄窗长输出。
