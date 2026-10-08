# macOS remaining feature and interaction parity

Baseline: merged PR #1480 (`b77b170d`). The user requested the remaining feature
gaps and interaction details on 2026-10-08.

- [x] Hooks: built-in defaults, command hook create/edit/toggle/remove, review the
  exact project hooks file and trust/revoke through the existing host commands.
- [x] Research journey: scoped relationship view, manual journal entries, daily
  recap review/edit/confirm/dismiss with source references, and run details.
- [x] Assistant: shared automation template shortcuts and inline calendar at
  wide widths, retaining usable narrow-window access.
- [x] Workflow: port connections, double-click blank-space creation and durable
  local canvas layout/camera preferences keyed by database, project and template.
- [x] Regression: scope isolation, stale replies, uncertain writes, immediate
  Escape, persistence and bilingual/light/dark rendering; full repository gates.

Use existing Rust commands and store validation. Do not execute hooks, schedules,
external models or real remote jobs during tests. Research relationship editing
is not currently offered by WebView; this increment matches its relationship
inspection and source navigation rather than introducing a new graph editor.

## Manual acceptance

Used the signed native QA bundle with the isolated database at
`/private/tmp/wisp-ui-alignment-20261003/native/wisp.sqlite`; production data was
not used. Created only synthetic records and configuration, without sending a
model prompt or executing a hook, automation, remote job or workflow.

- Hooks: saved a harmless command configuration, disabled it, reopened it, and
  pressed Escape without changing focus; the editor closed and settings stayed.
- Workflow: dragged the supporting node's output into another input and observed
  the new dependency/stage. Escape cancelled a pending connection without closing
  the canvas. Moved a node, zoomed to 83%, saved and reopened; positions, zoom and
  dependencies persisted. Double-clicked blank space at 83%; the new node was
  centred under the pointer. Closing protected the unsaved node with a discard
  confirmation.
- Journey: saved a Chinese/Unicode manual entry and saw it on the selected day.
  Edited and confirmed the synthetic daily recap. Its run source displayed the
  correct synthetic stdout; Escape closed only run details and retained the
  recap editor. Relationship details opened and closed independently.
- Assistant: the wide window showed project/plan, conversation and calendar.
  Literature-watch preset prefilled the canonical prompt, Monday and 09:00;
  saving remained disabled until choosing a project. Discarding its draft left
  automation management open and did not create a schedule.
- This acceptance caught raw enum/field names and Unix timestamps in the new
  journey views. The follow-up polish translates statuses and labels and formats
  dates using the current calendar/time zone. The rebuilt bundle was reopened
  and these labels/times were verified. The edited recap retained its source
  record while clearing citations on the rewritten item; the QA database
  confirmed both. The helper was stopped after acceptance and all schedules
  remain disabled with no schedule runs.
- English rendering also caught untranslated shared composer labels. The send
  button, follow-latest toggle, model placeholder and input hint now use the
  existing translation table.

## Boundaries and follow-ups

- Layout/camera preferences are local to this device and isolated by database,
  project and template; they are not exported as executable template content or
  synchronized with WebView/another device.
- Relationship browsing matches WebView's inspection scope. A manual relationship
  editor remains a separate product feature.
- Actual external model, hook execution, remote compute and cross-device sync
  were not part of this UI acceptance; fake services and existing backend tests
  cover the relevant contracts. No installer/release was published.

## Verification

- Swift full suite: 614 tests passed (595 UI + 19 core), including opt-in WebKit
  document rendering and Chinese/English, light/dark and narrow/wide snapshots.
  The final shared-composer label pass also passed the same complete suite.
- Native build and strict signature verification passed; packaging routing tests
  passed (6 tests), and the contract-generator regression passed (1 test).
- Rust journey command regression: 5 passed. The workspace unit/integration
  suite passed 2,682 tests. Its final Tauri doc-test compilation hit E0463 while
  QA packaging was also using the shared build directory. After packaging ended, the
  complete `cargo test --workspace --doc` rerun exited 0. Both original and
  retry logs are retained; the original whole command is not reported as green.
- wasm check passed. Formatting and native design/command-contract sync passed.
- `npm ci` completed. Full Playwright: 1,029 passed, 2 skipped and 2 tutorial
  expectation failures. Mainline commit `b145b04e` had added the remote-web
  tutorial without updating the explicit directory/order expectations. Those
  assertions were updated in a separate test-only commit; the complete tutorial
  group passed all 30 tests on rerun. The original full-run failure remains
  recorded.

Local verification evidence:
`/private/tmp/wisp-polish-swift-accepted.log`,
`/private/tmp/wisp-polish-rust-full.log`,
`/private/tmp/wisp-polish-rust-doc-retry.log`,
`/private/tmp/wisp-polish-playwright-full.log`,
`/private/tmp/wisp-polish-tutorial-retry.log`,
`/private/tmp/wisp-polish-wasm.log`,
`/private/tmp/wisp-polish-package-accepted.log`, and
`/private/tmp/wisp-polish-render-20261008/`.
