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
| Settings navigation | Eighteen enabled categories, project scope selector, scoped child models and unsaved-change guards. Channels and project settings remain. Confirmation and OAuth cancellation controls stay above the scrollable editor. |
| General / session / appearance / pet | Preference editors, local environment/network/update flows, pet controls and appearance font/CSS editing implemented. Full native font application and real-host acceptance remain. |
| Models / credentials | API presets, profile editing, exact catalog lookup, API test, default selection/reorder/remove and ACP configuration/test/authentication implemented. Model keys are separate host keyring arguments; image/video assignments use explicit top-level fields. Authentication terminal input is scoped, not persisted or automatically replayed, and late snapshots are ignored after disposal. Credential status and secret writes use host keyring APIs without secret readback. Real provider and ACP authentication acceptance remain. |
| Quick actions / workflows / specialists | Native create/edit/copy/remove, quick-action template binding and enablement, workflow tasks/dependencies/executor/budget editing, built-in read-only templates, reviewable skill/template conversions and specialist/reviewer configuration implemented. Null capability inheritance is distinct from an explicit empty whitelist. Host validation retains rejected drafts. Real-host conversion and persistence acceptance remain. |
| Plugins / browser / channels / permissions | Plugin install/toggle/remove, browser lifecycle and URL filters, and approval grant controls implemented. Channels and real-host acceptance remain. |
| Environments / storage / usage | SSH/WSL/context management, interpreter/storage preferences, retention and usage pages implemented. Real-host acceptance remains. |
| Publication / research editing | Audit SwiftUI behavior and complete publication evidence binding and journal/artifact/run editing. |
| Real host and model acceptance | Complete the smoke sequence above using disposable data, including real attachment copying, scratch lifecycle and queued model follow-ups. |
| Final regression and delivery | Re-run relevant suites for the final change, inspect real Windows layouts and layered Escape, document exact results, and deliver a focused follow-up PR. |

Current follow-up checks: Release WinUI build/publish passed; all existing C#
checks plus 28 settings-editor checks passed (draft retention, no retry,
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
