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

## First increment validation (#1355)

`dotnet run --project apps/windows/Wisp.ProjectBrowser.ContractTests -- contracts/project-browser/v1/projects.json`
runs existing protocol/navigation/conversation checks and 35 added parity model
checks. Saved-layout tests also cover revealing a closed Files tab. The parity
checks cover delayed and failed privacy reads, filtering every
calendar payload, latest-day selection, closed-view replies, draft retention,
group moves, scratch scope, attachment-only sends, per-session attachments,
queue guards, explicit acknowledgement, feedback and paused polling.

Release WinUI publish, native design/contract synchronization, Rust formatting
and the wasm UI check passed locally. Full `cargo test --workspace` reached the
Windows Tauri test executable, which exited before assertions with
`0xc0000139 / STATUS_ENTRYPOINT_NOT_FOUND`; the full Rust suite is not green.

Full Playwright regression finished with 867 passed, 2 skipped, and one timeout
in the existing WebView queued-guidance menu test: the target detached while
Playwright was waiting to click it. A focused rerun of all seven tests in
`queued-guidance.spec.ts` passed. The initial full run remains recorded as a
failure; the failure did not reproduce in the focused rerun. No WebView source
or test was changed for this Windows native increment.

Actual WinUI window checks use an isolated synthetic database and loopback
fixture host. They exercise the real native controls and client transport, not
real model requests or production host writes. Confirmed journeys include home,
create form, calendar privacy filtering, immediate Escape, project entry,
capability summary/detail with one-layer Escape, group form Escape, publication
content with the sidebar retained, and a library filter dropdown whose first
Escape keeps the library open. Do not interpret these checks as production create/import,
attachment-copy, scratch-cleanup or model/queue acceptance.

## Remaining real-host smoke

Use a disposable workspace and a current host build; do not reuse live research
data. Verify create and ZIP import appear after refresh; group/rename/move survive
reopening; attach a local file, change sessions and return, then send and reload;
queue exactly one draft during a real turn; and open/close scratch once. On each
screen press Escape immediately after opening, with a menu/dropdown open and
with a child sheet above a parent. Repeat at 150% scaling and a narrow window.

## Remaining parity work (active)

The follow-up objective is full completion of these remaining parts, not merely
enabling their entry points. Each row needs implementation and matching tests
plus real Windows UI/host evidence before it can be marked complete.

| Requirement | Current evidence / next work |
| --- | --- |
| Skills settings | Local import, enable/disable, tags, file inspection, removal, community catalog, pinned GitHub preview/import and native path pickers implemented. Real-host install acceptance remains. |
| Connections settings | Connector enable/approval controls, MCP stdio/HTTP editors, secret rows, OAuth command routing and cancellation implemented. Real host save/test and OAuth UI acceptance remain. |
| Memory settings | File/global memory editors and failure-analysis settings implemented. Synthetic-host edit/save and draft Escape checks passed. Project scope selector added after that smoke; selector and actual host writes still need verification. |
| Settings navigation | Nineteen enabled categories, project scope selector, scoped child models and unsaved-change guards. Project settings open from the home card and project menu, with explicit project identity and summary refresh after closing. Confirmation and OAuth cancellation controls stay above the scrollable editor. |
| General / session / appearance / pet | Preference editors, local environment/network/update flows, pet controls and appearance font/CSS editing implemented. Full native font application and real-host acceptance remain. |
| Models / credentials | API presets, profile editing, exact catalog lookup, API test, default selection/reorder/remove and ACP configuration/test/authentication implemented. Model keys are separate host keyring arguments; image/video assignments use explicit top-level fields. Authentication terminal input is scoped, not persisted or automatically replayed, and late snapshots are ignored after disposal. Credential status and secret writes use host keyring APIs without secret readback. Real provider and ACP authentication acceptance remain. |
| Quick actions / workflows / specialists | Native create/edit/copy/remove, quick-action template binding and enablement, workflow tasks/dependencies/executor/budget editing, built-in read-only templates, reviewable skill/template conversions and specialist/reviewer configuration implemented. Null capability inheritance is distinct from an explicit empty whitelist. Host validation retains rejected drafts. Real-host conversion and persistence acceptance remain. |
| Plugins / browser / channels / permissions | Plugin install/toggle/remove, browser lifecycle and URL filters, and approval grant controls implemented. Channels now include project sync, Feishu/Lark and Weixin binding, owner controls and device bridge configuration. Binding polls are explicit, honor host retry intervals and retain failed cancellations. Real binding, sync and host acceptance remain. |
| Environments / storage / usage | SSH/WSL/context management, interpreter/storage preferences, retention and usage pages implemented. Real-host acceptance remains. |
| Publication (#1342) | Initial creation and selected publication/revision/items are implemented. The initial form appears only after an empty workspace read and disappears after confirmed creation. Real-host persistence acceptance remains. PR #1342 explicitly excludes evidence binding, readiness and reproduction; unrelated research editors are not requirements introduced by this PR range. |
| Real host and model acceptance | Complete the smoke sequence above using disposable data, including real attachment copying, scratch lifecycle and queued model follow-ups. |
| Final regression and delivery | Re-run relevant suites for the final change, inspect real Windows layouts and layered Escape, document exact results, and deliver a focused follow-up PR. |

Current follow-up checks: Release WinUI build/publish passed; all existing C#
checks plus 29 settings-editor checks passed (draft retention, no retry,
duplicate-save exclusion, scope, argument casing, void success, unknown fields,
credential references, OAuth dispatch/cancellation, keyring writes, optional
plugin checksums, whitelist inheritance, independent copies, task identities,
conversion provenance, workflow validation failures and closed-view replies). The two Rust
native-settings DTO tests and contract sync check passed. This is interim
validation, not proof that the full parity objective is complete. An additional
16 model/authentication checks passed, covering key separation, assignment flags,
unknown settings, blank-key preservation, draft testing, full-list reorder,
late API-test cancellation, scoped terminal input, control-sequence stripping,
uncertain-input guards and closed-terminal replies. Release build/publish and
contract synchronization passed after adding models. Actual WinUI
fixture-host checks also confirmed the general preference form opens in the
same window and immediate Escape closes only its editor. Additional actual
WinUI checks confirmed quick-action editor Escape, project selection followed
by workflow reads and one save scoped to `parity-a`, and task-graph preservation
in that save. A long-editor discard confirmation was initially below the fold;
it now stays beneath the heading. Immediate Escape opens it, then another Escape
closes only the confirmation and preserves the changed workflow draft.
Dirty-draft navigation across scopes still needs UI acceptance. The full Rust
and Playwright results above belong to the first increment; they were not rerun
for this settings follow-up.

Model UI smoke used the same isolated fixture host: the API list/presets,
new-model form, ACP list and executable/argument editor rendered in the current
window. Immediate Escape returned each editor to its parent page. No real key,
external API test or authentication flow was exercised by this UI smoke.

Channel and project follow-up: Release build/publish and the complete C# harness
passed, including 13 channel checks for flow identity, scope, throttling,
cancellation, expiry, late replies, argument casing and unrelated settings
preservation. The project editor check verifies immutable project identity and
Agent Context argument casing. QR images use the host-returned SVG data; real
QR rendering/scanning and account binding have not been accepted yet.

Actual WinUI fixture checks verified the home card opens the selected project's
settings, immediate editor Escape returns to its parent, and one project rename
sends the explicit `parity-a` identity while preserving Agent Context. Returning
to home refreshes both the project card and recent-session labels. These are
synthetic-host checks, not production Rust persistence acceptance. Channel UI
navigation and dirty scope switching still need visual acceptance.

### Native conversation typography

Confirmed appearance preferences now update native conversation prose, Markdown
headings, code blocks, inline code, tool output, approval previews and the
composer. Home/sidebar text produced by the shared text helper also scales.
UI and code sizes remain independent; headings retain their relative sizes.
The appearance preview uses the same calculation, and confirmed preferences
are cached for restart. Editing the preview does not apply unsaved fonts.
An unchanged host read refreshes the cache; a dirty draft is not applied.

Six additional model checks cover older preferences, proportional headings,
independent code sizes, custom families, malformed values, bounds and wire
numeric values. Remaining typography acceptance includes actual Windows
rendering after save/reopen, all settings and auxiliary controls, narrow layouts
and scaling. This increment does not establish full native font coverage.


Publication scope audit: [PR #1342](https://github.com/xuzhougeng/wisp-science/pull/1342)
and `NativePublication.swift` establish an initial-create/read workspace, not a
full publication editor. The WinUI page now renders the host-selected paper
with its revision/items instead of presenting all paper titles above one
revision. The creation form waits for an empty workspace and disappears after
success. Lifecycle checks cover unread/existing/empty states, invalid drafts,
duplicate pending writes, confirmed creation and late closed-view replies.
Actual UI and production host persistence acceptance remain outstanding for
this adjustment. Earlier references to evidence binding and general research
editors as remaining requirements were broader than the requested PR scope.

### Isolated production Rust host acceptance (2026-09-23)

Built the current Rust desktop host with `TAURI_CONFIG` identifier
`science.wisp-science.parity-acceptance` and `WISP_CATALOG_OFFLINE=1`, then ran
`--native-settings-host` against its separate app-data database. Native broker
requests used the real Rust implementation and real Tauri invoke, not the
synthetic fixture server. The user research database was not used.

Verified project creation, initial publication creation/read, group creation
and rename, project name/Agent Context save/read, scratch open/close, native
conversation creation, attachment copying with exact-byte comparison, and
moving the conversation into the group. After restarting this isolated host,
publication identity, renamed group, project settings, session group assignment
and attachment bytes remained intact. Session storage verification reads the
project's `.wisp/project.sqlite`, not the application registry database.

This is real-host protocol/persistence acceptance, not WinUI visual acceptance.
ZIP import, attachment send/reload through a model turn, queued follow-ups,
credential/provider/OAuth/channel flows and narrow/150% layouts remain open.
Current formatting, wasm check and native design/settings synchronization pass.
A fresh full Rust workspace run is underway. The first fresh Playwright attempt
failed during test-server startup before tests ran; a separate Trunk build
succeeded, and a full rerun uses those assets on an isolated static server.
Neither full suite has a final passing result recorded yet.

ZIP import and real WinUI follow-up: a disposable valid ZIP fixture containing
current-schema project metadata and one workspace file was imported through
`native_project_import`; the returned project identity, extracted bytes and
project settings readback passed. The current WinUI acceptance build connected
to this real host and displayed both the imported project and the earlier
created project. Opening the existing publication showed its title,
description and `v1 / draft`, with no initial-create form. Immediate Escape
returned to the project conversation area while retaining the sidebar.

The fresh full Rust run stopped in `wisp-runs`: 171 passed and 2 failed.
`auto_harvest_skips_collect_when_already_harvested` reported SQLite database
locked; `ssh_input_staging_ledgers_uploaded_files` expected one ledger entry
but observed zero. Focused reruns are underway; neither failure is waived.
The full Playwright rerun is still running on the separately built frontend.
Its research-journey screenshot tests regenerate tracked design-QA PNGs;
these generated files are not part of this native change and must be restored
after the test run finishes.

Both failing `wisp-runs` tests passed in focused reruns. The original full-run
failure remains; this does not establish a fully green workspace suite.
A remaining-workspace run excluding that package is now checking the crates
that the first run did not reach.

Real WinUI inspection also found recovery buttons visible on a normal empty
conversation. The retry button now appears only for a reported error, and the
acknowledgement button only when a send/queue result is uncertain. Release
build/publish and the C# harness passed; a second real-host WinUI launch
confirmed neither recovery action appears on the normal empty project page.

### Expanded native font bindings

Settings navigation/forms, shared sheet headings and notices, search results,
conversation controls, auxiliary panels and terminal controls now bind to the
committed native font preferences. The binding pass visits app-owned content
only, preserves explicit renderer fonts and existing bindings, and does not
traverse generated templates or icon glyphs. Repeated refreshes therefore do
not repeatedly scale already configured text. Appearance preview content is
excluded so unsaved changes remain preview-only. Terminal input/output use the
independent code font setting.

Actual WinUI acceptance against the isolated Rust host set UI/code sizes to
20/18, confirmed enlarged home text, settings headings, navigation buttons,
combo boxes and labels at the approximately 800x565 logical-pixel window, then
restored 14/12 and reloaded the same mounted settings page. Both text and
controls shrank immediately without reopening, and headings retained their
relative size. The original isolated-host preferences were restored. Release
build/publish and the complete C# harness passed; remaining surface-specific
layout and dirty-navigation acceptance are still tracked above.

The remaining-workspace Rust run reached Tauri after the preceding crates
passed, but its unit-test executable exited before assertions with
`0xc0000139 / STATUS_ENTRYPOINT_NOT_FOUND`. This confirms the local test-loader
problem still exists; the separately built production host acceptance above
passed and is not evidence that the Tauri unit suite ran successfully.
