# Windows native parity: PRs #1332–#1351

The WinUI preview now connects the native workbench actions introduced in this
range to the existing Rust desktop host. No second database writer or WebView
project activation was added. Build with `scripts/build_native_windows.ps1`.
An alternate browser database still requires a matching running host descriptor.

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

All 19 settings categories open in the existing WinUI window. Native editors
cover general/session/appearance/pet preferences, skills and SkillStore,
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
