# WinUI / WebView remaining interface parity

Requested 2026-10-05: complete all gaps in the comparison against `main` at
`96108e59`. This is the full acceptance ledger; completing a batch does not
complete the overall request. Existing native capabilities must stay intact.

## Required outcomes

- [ ] Shared interaction preferences: Enter/Ctrl+Enter, IME, selection actions,
  and one persisted theme across the footer, Settings and retained controls.
- [ ] Composer references: @/# file/artifact/session/project/context references,
  / skill/workflow selection, stable reference chips and source-aware quotes;
  matching commands, context usage and follow-up questions.
- [ ] ACP: create/select configured agents, render agent modes/configuration,
  exact permission/question responses and session binding restrictions.
- [ ] Conversation actions: branch, review, rewind/undo and turn memory, with
  correct historical turn identity; keep read-only actions usable during runs.
- [ ] Queued turns: visible authoritative queue, edit/cancel/cut-in/replace
  actions, uncertain-write handling and session-isolated drafts.
- [ ] Reading: structured tool results, live inline Run cards, plan progress,
  automatic results-review prompts and persistent dismissal semantics.
- [ ] Research assistant: cross-project conversation, project/calendar sidebars,
  remote access, automation management and conversation timer UI.
- [ ] Preview: DOCX/XLSX/PPTX, molecule/structure/MSA, isolated MCP Apps, and PDF
  text selection with source-aware add-to-chat; retain size/cancellation limits.
- [ ] Search and commands: message full-text search, command palette, Codex and
  Claude/session-archive import with preview and project-scoped destinations.
- [ ] Publication: item editing, evidence binding, revision management and
  reproduction controls equivalent to the current WebView workspace.

## Verification gates

For each outcome: inspect the current WebView behavior, extend shared Rust DTOs
and scoped native dispatch as needed, add substantive contract/model regression
tests, build the WinUI preview and exercise real controls. No real SSH, provider,
GPU or network dependency in automated tests. Writes must not be replayed after
ambiguous responses. Late reads must not replace a new project/session/preview.
Escape closes only the uppermost surface immediately after opening it.

Run relevant focused checks first; after implementation run the required Rust
format/workspace, WASM, Playwright and applicable MCP smoke suites. Package the
matching native host and resources. Validate both clients at equivalent window
size, DPI and typography, including narrow windows, theme changes, retained
drafts, navigation, cancellation and focus. Record failures and unverified
acceptance separately; successful model tests are not real-window acceptance.

## Execution record

- Initial audit: current checkout is `main` at `96108e59`; no tracked local
  changes. Existing `.zcodeignore`, `test-results/` and `ui/dist-test-probe/` are
  unrelated and remain outside the implementation.
- First batch in progress: shared appearance persistence and send preferences.
- Implemented host-persisted footer theme updates and saved send-key semantics;
  native regression suite and WinUI Release build passed. Selection popup and
  live-window acceptance remain outstanding.
- Implemented typed @/# and skill/workflow candidates, session draft chips and
  send/queue transport. WebView and native now share runtime candidate capability
  rules and the reference wire enum. Native model/contract regressions passed,
  including stale reads, identity rejection and uncertain sends. Native host
  tests are compiling; full regression and real-window checks remain pending.
- Added session-owned quote chips with absolute source-turn identity, selection
  actions, context-usage buckets and append-only follow-up suggestions. Follow-ups
  use a bounded transient host cache and disappear when their session starts a
  new user turn. Quote text is bounded individually and in aggregate. Selection
  popup and context usage still need live acceptance.
- Added configured ACP conversation creation, exact permission IDs/cancellation,
  and direct responses to waiting questions. Bound ACP sessions cannot be
  switched to an HTTP model. Added mode and grouped select/boolean controls;
  changes validate current advertised choices at both client and host and never
  replay on an uncertain response. Runtime snapshots retain current mode updates
  while restarted sessions show only cached available modes.
- Live isolated fixture checks verified @ filtering, Escape retaining the draft,
  Enter selecting a reference without sending, and source identity on the 35th
  turn's quote. These checks exposed and fixed a VirtualKeyStates enum mismatch
  that crashed keyboard input and a bubbling KeyDown handler that allowed the
  multiline editor to consume Enter first. Main and auxiliary editors now use
  PreviewKeyDown with the saved send preference and IME guards. Auxiliary typing
  still needs a live check of the latest build.
- Current focused checks: native C# contract suite and WinUI Release build passed
  (zero warnings/errors); shared conversation DTO tests passed (31); synthetic
  ACP peer tests passed (3). Native Rust host tests passed (78); WASM check passed
  with the existing four Leptos dependency warnings. Complete workspace,
  Playwright and real-provider acceptance are still pending.
- Real isolated ACP acceptance: mode and grouped select changes reached the
  synthetic peer as exact IDs and were confirmed by the host snapshot. Consecutive
  mode/config changes keep the parent flyout and selector instances alive. Do
  not disable the ComboBox synchronously from SelectionChanged: WinUI is still
  closing its popup and doing so breaks the next nested popup. The model's Busy
  guard serializes mutations. Immediate Escape closes only the dropdown, then
  a second Escape closes the parent configuration flyout. The compact scrollable
  flyout preserves the chat area at the observed 801x567 logical window size.
- A real host restart exposed a stale port/token in NativeSettingsClient. Each
  new request now reads and validates one paired descriptor before sending; it
  never retries the current attempt. Regression tests cover endpoint rotation,
  no write replay, foreign database rejection and non-loopback rejection. Live
  restart recovered the transcript and retained an unsent synthetic draft.
- QA used an isolated app identifier/database and a temporary startup-only
  fixture injection in the published QA binary. App.xaml.cs was restored with
  zero source diff. This does not claim normal production startup, real ACP
  authentication, live permission/question UI, or complete interface parity.
- Added historical message menus for before-user/after-response branches,
  session review, rewind/edit, latest-turn undo and editable project/global
  memory proposals. Requests bind project/session, absolute turn index, durable
  user sequence and content digest. Rewind/undo confirmations additionally bind
  the transcript revision; native rewind claims the workflow lock before
  validation. Legacy sequence-zero rows remain valid. Historical memory reads
  keep Stop available during a later running turn; destructive actions do not.
- Native history contract regressions cover page offset 34, running versus
  historical eligibility, stale-row rejection, late navigation responses,
  foreign reply identity, preserved drafts, sequence-zero legacy records and
  no replay after ambiguous writes. Full C# harness and clean WinUI Release
  build passed. Full DTO suite passed (78); WASM check passed with the four
  existing Leptos dependency warnings. Native-filtered host tests passed (80),
  including replaced identities and workflow exclusion; complete
  workspace/Playwright remain due.
- Isolated real-window history checks passed at 801x567 logical size: user and
  assistant menus, immediate Escape on menu/rewind dialog/undo preview, restored
  before-user branch draft, and disabled branching of an existing branch.
  Source remained 35 turns; the UI-created branch contained 34. A separate
  after-response branch created through the real host contained 34 turns; undo
  reduced only that disposable branch to 33, with no file/artifact effects.
  Cross-project, replaced-turn and stale-confirmation requests were rejected.
  Provider-backed review/memory generation and the memory editor's nested
  replacement dropdown still need live acceptance. UI deletion/undo confirmation
  was not clicked; destructive behavior was checked through the isolated API.
- Added the authoritative shared queue projection, decimal-string IDs, payload
  digests, exact edit/cancel/reorder/cut-in/replace actions and up to 64 native
  follow-ups. The native client reconciles lost enqueue replies by ID, preserves
  independent drafts after navigation, retains failed/unconfirmed payloads for
  explicit recovery, and never replays an uncertain mutation. Completed rows
  are retired even when acknowledgement arrives later. A replacement reservation
  is checked again under the queue lock after workflow acquisition, preventing
  the driver from starting the replacement before Stop targets the old turn.
- Queue validation: full C# contract harness passed; full shared DTO suite
  passed (79); Rust queue-filtered host tests passed (22). Clean WinUI Release
  build passed with zero warnings/errors; fmt, diff and native-settings contract
  synchronization checks passed. Full workspace/Playwright remain outstanding.
- Real isolated ACP queue acceptance at 801x567 logical size: a native composer
  enqueue and two API enqueues appeared as three rows; UI move-up and editor save
  matched the authoritative host state. Immediate Escape closed both the menu
  and editor without changing queue payloads. ACP cut-in was disabled. API
  cancellation removed only its target; stale replacement was rejected without
  stopping the active turn. Valid replacement ran the selected message first;
  subsequent edited and layout-fixture rows drained in order. Four completed
  outcomes and one cancelled outcome were verified against the transcript.
- Short-window QA exposed excessive default Expander padding. Replaced it with
  a compact accessible header, shared chevron icon and 80px/160px responsive
  scroll area. Real collapse/expand, scrolling and queue disappearance after
  completion were checked in the latest QA binary. Host restart retained the
  unsent draft and recovered the connection. App.xaml.cs remains restored.
- Queue limitations: HTTP-agent cut-in and live lost-response/restart recovery
  still have automated coverage only; no real provider was contacted. QA used
  an isolated host/database and temporary startup injection, not production
  packaging. The sidebar session badge still showed completed during the ACP
  running fixture; add live sidebar status refresh to remaining integration QA.
- Reading implementation: shared accepted-plan parser/projection and composer
  progress, ACP sections with stable call identities, exact session-owned Run
  records/output tails, inline progress/actions and model-owned review prompts.
  Duplicate ACP IDs get separate row keys. Late prompt checks revalidate current
  ownership/status/cleanup/read-only state. Cancel and dismissal mutations are
  never replayed after ambiguous responses. Automatic prompts defer behind
  existing root and conversation overlays; Escape closes the prompt first.
- Reading checks passed: C# contract harness including rejected/pending plans,
  duplicate ACP IDs, real byte progress, no-replay cancellation, busy-to-idle
  review, stale/foreign/cleaned/read-only checks and late dismissal; native Rust
  filtered suite 84 tests; DTO suite 80 tests; WASM check with the four existing
  Leptos warnings. Clean WinUI Release build passed with zero warnings/errors.
  Native settings contract synchronization, fmt and diff checks passed before
  final documentation changes.
- Isolated real-window reading acceptance: the unfinished plan showed 1/4 with
  independent cancelled status and preserved continuation text; expand/collapse
  worked. The completed plan showed 3/3 and its close action removed the strip.
  Failed ACP tool output retained a literal script-like string, path/line,
  terminal identity, and before/after code. This uncovered first-line truncation
  from assigning WinUI TextBox.Text before AcceptsReturn; initialization order
  was fixed and both before/after blocks were visibly rechecked in a new binary.
- A non-executing synthetic Run showed updated 75% byte progress and multiline
  stdout/stderr. On success, the root review prompt appeared; immediate Escape
  dismissed only the prompt. A subsequent scoped host check returned
  should_prompt=false, confirming persisted dismissal. The fixture carries a
  private lifecycle lease so the real manager cannot reclaim its nonexistent
  process; no real SSH or provider was contacted.
- Full workspace test attempt stopped at the previously recorded Windows CLI
  baseline: eval::tests::run_wait_case_completes_without_shell_sleep reported
  tool errors exceeded limit: 1 > 0 (43 CLI tests passed, 1 failed). This is not
  a green workspace run and later workspace packages were not reached.
- Reading remains open: plan-mode proposal/decision cards, additional rich
  tool renderers, continuously refreshed historical Run cards, direct automatic
  result-browser opening rather than the intermediate native prompt, manual
  result-browser dismissal semantics, live failure/rejection and uncertain
  cancellation journeys, and final packaged same-condition acceptance.
- Final reading QA binary confirmed terminal Run results start folded inside
  the exact submission and can be expanded to see 100% progress and complete
  multiline output. Terminal states have model regressions for succeeded,
  failed, cancelled, timed_out and lost; the live fixture exercised succeeded.
  Completed-plan dismissal also has model coverage across polls, navigation,
  identical next-turn plans, and incomplete-plan rejection.
- Final C# harness and clean WinUI build passed; the temporary App.xaml.cs QA
  injection was restored with no source diff. The active QA host uses
  target/native-windows-parity-reading-host and the latest QA client uses
  target/native-windows-parity-reading-final, both isolated from installed Wisp.
  Full Playwright (1007 cases) was restarted on port 14822 after the default
  port 1422 failed with address-in-use; results remain in progress in
  test-results/winui-parity-playwright-14822.log. Do not count that suite as
  passed until its completion result is recorded.

- Decision/status batch: added latest-turn native/ACP plan proposals with
  Markdown, statuses, priorities, approve and save/exit. Approval verifies mode
  exit before sending; save preserves the draft. Direct automatic Run review
  now opens the result browser, and closing manual/automatic reviews persists
  dismissal without replay after an uncertain reply. Sidebar runtime status
  updates in place without rebuilding the conversation or composer.
- Isolated live decision evidence: native Save/exit disabled plan mode while
  retaining the unsent draft; ACP Approve switched from plan to advertised brief
  mode and sent the exact draft. Sidebar changed to running, then completed after
  Stop while retaining a later draft. An eligible completed synthetic Run opened
  the browser directly; Escape returned to Run details and the host confirmed
  dismissal. Remote listing was unavailable because the fixture has no server
  workspace; no real remote-file browsing is claimed.
- Decision checks: native-filtered Rust 85 passed; DTO 81 passed; full C# harness,
  WinUI clean build (zero warnings/errors), WASM check and synthetic ACP peer
  tests (4) passed. Run-detail multiline initialization was fixed in addition
  to tool bodies; the PDF QA binary later visibly confirmed all five stdout
  lines and the stderr warning in Run details.
- Full Playwright finished with 992 passed, 2 skipped and 13 failed. Twelve
  failures were stale homepage button-count/layout assumptions; one exposed a
  lazy Automation project cache on first opening during a refresh. All test
  results had printed but the server child kept teardown alive; the exact
  task-owned Trunk on port 14822 was verified and stopped before terminal exit1.
- Corrected the Automation cache by eagerly observing completed project reads
  before the form is first mounted, retaining privacy filtering and stable form
  identity. Updated homepage assertions to the current eight actions and to
  check alignment/nonoverlap for either valid flex layout. The full Automation
  and wordmark files passed (20 tests); their task-owned Trunk on port 14823
  likewise needed verified shutdown for teardown to complete (terminal exit0).
- PDF selection batch in progress: native source-aware document quote cards,
  PDF.js text selection and Add/Add-and-jump controls; C# source/identity/bounds,
  draft recovery/send/queue checks and browser renderer coverage passed. Native
  build passed with zero warnings/errors. Live UI exposed a previously missed
  host mismatch: read_file_at extracted PDF Markdown instead of returning page
  bytes. Native previews now use a bounded raw-PDF path, with regression coverage
  against an extractable two-page fixture; host test/build and full live PDF
  acceptance are still pending. These changes do not complete the broad preview,
  assistant, publication, search/import or packaged acceptance outcomes above.
- PDF follow-through: the shared `render_pdf` request flag opts WinUI into raw
  page bytes while old/native text clients retain document extraction. The
  extractable two-page Rust fixture verifies byte fidelity, legacy text behavior,
  scoped reads and the 32 MiB rejection; the shared JSON fixture is checked by
  Rust and C#. Final native-filtered Rust suite passed 86 tests, DTO 81, full C#
  harness passed, and WASM passed with the same four dependency warnings.
- Real PDF acceptance at 801x567 logical size: rendered both pages; selected
  actual PDF text; Add kept the preview and created a source chip; Add-and-jump
  from page 2 preserved the existing Chinese draft and showed the exact path,
  page 2 and project. A live keyboard boundary defect was fixed by forwarding
  unconsumed WebView2 Escape into the existing native window stack. First Escape
  clears only the selection; second closes only the preview, retaining Files.
  Closing the narrow panel initially failed to focus the composer until layout
  re-enabled it; deferred, session-checked focus was verified in a fresh binary
  by typing immediately after Add-and-jump without clicking the input.
- Latest isolated PDF client is `target/native-windows-parity-pdf-quotes-focus`;
  host is `target/native-windows-parity-pdf-host`. QA source injection was restored
  after publishing. No provider or real SSH was used, and this is not production
  packaging acceptance. Browser PDF/KaTeX regression passed, including real mouse
  selection, page/zoom changes, rejected add, Escape forwarding, empty text layers
  and invalid documents. Assistant-workspace regressions passed (16); its verified
  port-14824 Trunk needed shutdown after assertions to reach terminal exit0.
  Native asset/settings synchronization, fmt and diff checks passed.
- Final PDF source build passed with zero warnings/errors after restoring
  App.xaml.cs. The isolated host snapshot still showed one original user message
  and no running turn after quote/add/jump/typing QA (no send occurred). Evidence:
  `test-results/winui-pdf-quotes-live-evidence.json` and the corresponding
  contracts, browser, host-tests-final, dto, wasm and clean-build logs.
- Broad regressions restarted after these fixes and are currently running:
  Playwright on port 14825, log `test-results/winui-parity-playwright-14825.log`
  (exec session 85510); supplemental workspace run with only the previously
  failing CLI `eval::tests::run_wait_case_completes_without_shell_sleep` excluded,
  log `test-results/winui-parity-workspace-except-known-cli.log` (session 36436).
  The earlier unfiltered workspace failure remains unresolved; the supplemental
  run cannot establish an entirely green unfiltered suite. Poll these exact
  running sessions before restarting or reporting completion.
- Office batch: native file/artifact requests now independently opt into
  validated DOCX/XLSX/PPTX bytes (`render_office`, default false, 32 MiB input
  ceiling). PDF-only and legacy clients keep their previous behavior. Both
  frontends use the same Office DOM/worker module; the native package includes
  its pinned renderer bundles, shared ZIP chunk, worker and licenses. Native
  previews retain exact model ownership, close/dispose on replacement and route
  Escape through the existing window stack. Office source quoting remains open.
- Office focused checks passed: native-filtered Rust 87 tests, full C# harness,
  native renderer browser suite (3), WebView document journeys (6), narrow PPTX,
  external XLSX references and build configuration (6). Coverage includes Word
  table/math/image rendering, bounded multi-sheet Excel with cached formulas,
  PPTX, corrupt archives, cancellation/replacement, legacy extraction, size and
  project boundaries. WASM passed with the existing four dependency warnings.
  Native source and QA publish succeeded with one cached NuGet audit network
  warning (NU1900); source QA injection was restored. Ten shipped Office assets
  and licenses were hash-checked against their sources. Live Office acceptance
  is still in progress against the isolated QA database.
- Supplemental workspace run ended with a failing wisp-store suite: 234 passed,
  19 failed, 1 ignored. Nine credential tests reported Windows access denied,
  nine session-artifact tests failed, and one archive test hit Windows file
  sharing error 32. This does not resolve the excluded CLI baseline and is not a
  green workspace suite. Broad Playwright on port 14825 is still running; it
  overlapped the Office module extraction/runtime synchronization and has
  failures, so its result cannot establish a clean final-source regression.
  An additional source-highlighting case failed there but passed its focused
  unchanged rerun. Final stable-source full regression remains required.
- Office live follow-through at 801x567 logical size: Word page/table/fraction,
  Excel bounded-data notice and two sheet tabs, and the narrow PPTX slide rendered.
  Selecting the second sheet showed its own content; horizontal scrolling to
  the cached value 84 and clicking it displayed `=B2*2` in the formula bar.
  Escape from a focused workbook closed only the preview and retained Files.
  Live checks found two defects: newly opened documents were below the file
  list, and the native Office stylesheet initially omitted its shared layout
  rules. New document previews now reveal their source/viewport after layout,
  guarded against replacement; ordinary refreshes do not reposition the reader.
  Restored shared layout rules and added real page-boundary and virtual-grid
  geometry assertions. The full native browser suite passed again (3).
- Final shared DTO suite passed 81 tests after correcting a test fixture's
  missing required session ID. Full C# harness and final source publish passed;
  the publish retains the cached NU1900 audit-network warning. The restored
  App.xaml.cs has zero diff, final source output contains the corrected stylesheet,
  and fmt/diff checks pass. Current isolated Office client:
  `target/native-windows-parity-office-visible`; host:
  `target/native-windows-parity-office-host`. API and live observations are saved
  in `test-results/winui-office-live-evidence.json`. No real provider/SSH was used.
  Full-source suite verification, Office source quoting, scientific/MCP previews,
  research assistant, search/import, publication and the other ledger gates remain.
- Scientific preview batch: both clients now use a shared offline renderer for
  molecules, PDB/mmCIF/MOL2, FASTA and FASTA/CLUSTAL/Stockholm alignments. RDKit
  runs in a disposable worker; fixed local structure/alignment documents own
  their vendor components. Native origin filtering also covers worker requests.
  Invalid, oversized and truncated inputs fail explicitly. Source-text mode
  retains editing and can return to interactive preview. Fixed the pre-existing
  blank Nightingale integration by supplying records after initialization and
  setting a nonzero sequence length before its first render.
- Scientific follow-through found and fixed detached WebView listener/worker
  cleanup, and moved the native source/close actions above the viewer so they
  remain discoverable in a short window. Native renderer plus WebView file
  journeys passed 8 tests (`winui-scientific-browser-final.log`); the center
  structure/FASTA height regression also passed. The first journey run used an
  incorrect close-tab selector; after fixing that selector, all eight finished
  with terminal exit0. The initial test server needed verified task-only shutdown
  on port 14826 after its assertions; it is no longer running.
- Scientific live acceptance at 801x567: actual SMILES drawing, three-atom mmCIF,
  drag rotation, wheel zoom and Escape retaining Files were observed in the
  isolated WinUI client. The final client shows both source/close actions above
  the viewer; Stockholm visibly shows two named sequences with colored residues
  and a gap. Source mode displays the complete multiline original and returning
  re-renders the alignment. Escape after clicking its canvas retains Files.
  No source save or chat send was performed. Evidence is recorded in
  `test-results/winui-scientific-live-evidence.json`.
- Final scientific C# harness, WASM check, native design/settings synchronization,
  format/diff checks and restored-source publish passed. WASM retains four
  existing Leptos warnings; publish retains cached NU1900. Twelve authored
  scientific assets/licenses match their sources in both QA and source builds.
  App.xaml.cs has no QA injection in the source. Current isolated client:
  `target/native-windows-parity-scientific-final`; restored-source build:
  `target/native-windows-parity-scientific-source`; host remains
  `target/native-windows-parity-office-host`.
- Broad regression correction: the port-14825 Playwright run is terminal,
  991 passed, 2 skipped, 14 failed. It overlapped source changes and does not
  establish final-source regression. The supplemental workspace failures and
  excluded CLI failure recorded above remain unresolved. Full clean regression,
  scientific/Office source quoting, MCP Apps, research assistant, search/import,
  publication, historical Run refresh and same-condition packaged acceptance
  remain required; the overall goal is not complete.
- Search batch: added shared `wisp.native-search.v1`, direct scoped native host
  dispatch and a cancellable WinUI search overlay for projects, artifacts and
  session titles/message bodies. Visibility is applied before session limits;
  native session search now reuses exact global store ranking rather than
  concatenating per-project results. New tests cover both local and routed
  stores, more hidden hits than the result cap, truncated display titles,
  current-project preference and shelving.
- Ctrl+K supports exact-owner open, Shift+Enter reference attachment and
  Ctrl+Enter project/session opening in independent native windows. App retains
  all live windows, each with its own browser/conversation state and explicit
  database. Ctrl+Shift+P and a compact sidebar entry expose 23 implemented
  command routes, including new window, theme and side-panel actions. Search
  queries, errors and closed overlays cannot accept stale replies.
- Real search acceptance at 801x567: body-only Chinese query opened the exact
  Run session; two projects' same-name CSV results showed their owners;
  Shift+Enter added a CSV reference while retaining the Chinese unsent draft;
  Ctrl+Enter opened the exact Run session in another window and left that draft
  intact. Immediate Escape preserved the underlying Files panel. No send occurred.
- Live QA found and fixed missing command-icon exports, ItemClick returning
  container content, and ListView consuming Enter before its bubbling handler.
  Added command-registry icon validation to the native export check and C#
  harness. The final build's mouse click opened the other project's original
  counts.csv, and Tab to the command result followed by Enter switched to Files.
- Search checks: native Rust 88 passed, shared DTO 82 passed, full C# harness,
  local/routed ranking tests (2) and shelving tests (4) passed. Native source
  and QA publishes succeeded. Initial NuGet failure was the sandbox account's
  empty package cache; explicitly using the existing user's package cache fixed
  restore without downloading dependencies. Source App.xaml.cs retains only
  the real multi-window change, with no temporary QA database injection.
- Current isolated host: `target/native-windows-parity-search-host`; final QA
  client: `target/native-windows-parity-search-final`; restored-source build:
  `target/native-windows-parity-search-source`. The host uses a dedicated WebView
  data directory and the existing isolated parity database. Production Wisp was
  left running. Search/command live evidence is in
  `test-results/winui-search-live-evidence.json`.
- Search/command work remains incomplete as a full outcome: session imports,
  export/setup/privacy/update/font commands and other WebView routes are still
  open. Full stable-source regressions and same-DPI packaged acceptance remain
  required, together with all other unchecked outcomes above.
- Final search WASM check passed with the four existing Leptos dependency
  warnings; fmt/diff and both native design/settings synchronization checks
  passed. Five new shared icons match both final QA and restored-source outputs.
  The isolated Run session remains idle with its one original user message.
- Session ZIP import batch: added shared `wisp.native-session-import.v1`,
  direct project-scoped host preview/import, typed client/state and a WinUI sheet
  in both the project menu and command palette (now 24 routes). Preview lists
  message/artifact counts, snippets and existing-import status; source/destination
  changes invalidate it. Commit binds the reviewed SHA/source ID and extracts
  those same bytes. Scoped mappings preserve legacy same-project imports while
  preventing updates to a different project's imported conversation.
- Import writes preserve existing files, bound actual extraction, remove failed
  partial files, reject busy/frozen/read-only targets and never replay uncertain
  replies. Opening the confirmed result directly avoids a refresh continuation
  overriding a newer navigation. The existing WebView importer also benefits
  from project-scoped archive deduplication and bounded input parsing.
- Archive focused checks passed: 10 archive Rust tests, 92 native-filtered Rust
  tests, 83 shared DTO tests, full C# harness, WASM with the four existing Leptos
  warnings, source/QA publishes and format/diff/design/settings checks. Publish
  retains cached NU1900. Temporary App.xaml.cs QA injection has been restored.
- Initial live archive checks at 801x567: project-menu entry opens the sheet;
  immediate Escape retains the original Run session and unsent Chinese draft;
  Escape from the destination dropdown closes only that dropdown. Matching host
  build and actual preview/import acceptance are still in progress. This batch
  does not implement Codex/Claude discovery/import or complete the overall goal.
- Archive live follow-through: switching destination cleared the old preview;
  both projects remained unimported after read-only preview. Explicit import
  created exactly the selected other project's frame with two messages and one
  CSV; the original project still reported `new`. Both message lines and the
  artifact list were reachable by scrolling in the 801x567 window. Opening the
  result selected the exact destination/frame. Malformed ZIP retained its path
  and disabled import. Replacing a source after preview was rejected before
  mutation, and re-preview only exposed an existing-session inspection route,
  never another write. That inspection opened the correct existing frame.
- Live result review exposed stale "will create" wording after success; the
  final client shows confirmed archive contents instead. Uncertain state now
  retains an explanation even after a successful reconciliation preview. Final
  client and final host skipped a repeat import with the same frame identity,
  displayed the corrected result, and preserved CSV contents. Destination root
  is now resolved again under the project mutation guard after archive parsing.
- Final archive source: 10 archive and 92 native-filtered tests passed again;
  C# harness and restored-source publish passed. DTO83 and WASM checks above
  remain applicable. No temporary QA database injection remains in App.xaml.cs.
  Current host is `target/native-windows-parity-import-host-final`, client is
  `target/native-windows-parity-import-final`, restored source output is
  `target/native-windows-parity-import-source`. Evidence is
  `test-results/winui-session-import-live-evidence.json`. No real provider,
  SSH host or production database was used. Full stable-source regression,
  Codex/Claude import and all other unchecked outcomes remain outstanding.
- Codex/Claude import implementation: expanded the shared session-import
  contract with source/list/preview/import, reusing bounded local/WSL/SSH readers
  and metadata caching. WinUI now has explicit destination/provider/source,
  title/cwd/ID/path filtering, 25-row pagination, reviewed single import and
  serial filtered bulk import across pages with stop-after-current and separate
  outcome counts. Preview/import bind full-source SHA and exact identity;
  ambiguous writes stop without replay. Closing or changing a read selection
  rejects late results. Project-scoped legacy-aware deduplication also fixes the
  existing WebView importer updating a different project's conversation.
- External import checks so far: 18 Codex/Claude Rust tests, 10 archive regression
  tests, 95 native-filtered host tests, 84 DTO tests, full C# harness (including
  delayed preview after filter/page changes and bulk import across pages), WASM
  and restored-source/QA WinUI publishes passed. Publish retains cached NU1900;
  WASM retains the four Leptos warnings. Real-window external-import acceptance
  and matching isolated host build are in progress; no complete parity claim.
- External-import live follow-through at 801x567: project menu and command
  palette opened the sheet; immediate Escape preserved the originating draft,
  and destination-dropdown Escape closed only that selector. The 27 synthetic
  Codex sources paged to two final rows; ID/cwd filters worked, destination
  changes cleared previews, and preview alone left both projects unimported.
  Single import created two multiline messages only in the selected other
  project and opened the confirmed frame. Claude bulk import reported 4/4
  created, zero failures and no remaining bulk action. Appending two synthetic
  messages then rescanning produced one updatable row; the next import reported
  one update, preserving its frame ID and leaving the original project untouched.
- Real-window QA exposed a source-list contract mismatch for a display-only
  local execution context. The host now offers only IDs/kinds accepted by the
  source reader and guarantees the canonical local entry; mismatched remote
  kinds are rejected too. The regression passes, and the final QA host no longer
  shows the source-list error. Final checks passed: Codex/Claude Rust19,
  native-filtered96, full C# harness, WebView mocked import2, format/diff and
  design/settings synchronization. DTO84, archive10 and WASM above remain
  applicable. The two Playwright tests finished exit0 after verified shutdown
  of their test-only Trunk server. Both QA and restored-source host builds passed.
- External-import evidence: `test-results/winui-external-import-live-evidence.json`;
  host `target/native-windows-parity-external-host-final`, client
  `target/native-windows-parity-external-qa`, restored WinUI source output
  `target/native-windows-parity-external-source`. Temporary source substitutions
  were restored. QA used a synthetic CLI home and an isolated database; no real
  provider or SSH host was used. Real WSL/SSH scanning, slow-write stop/uncertain
  UI journeys and production packaging remain unverified (model/fake-runner
  tests cover their core semantics). This batch does not complete the unchecked
  assistant/publication/MCP/quoting/commands/historical-Run/full-regression goals.
- Historical Run batch: pinned historical pages now refresh their exact Run
  records with duplicate suppression and at most four concurrent reads. The
  transcript cursor, outline target and draft are retained; failed reads label
  the last confirmed record and disable cancellation. Page/session/epoch and
  request generations reject stale replies. Active historical cancellation uses
  the existing scoped path without replay after uncertain results. Completed
  historical Runs can nominate deferred review after returning to latest even
  when absent from the latest page, subject to fresh record/host eligibility.
- Outline entries now navigate the actual conversation and close the sheet,
  rather than appending a separate transcript inside it. Layout-aware reading
  targets position the selected question, older-page navigation starts at its
  first item, and latest resumes following. Closing the sheet rejects a late
  navigation reply even when its transport ignores cancellation.
- Real historical reading acceptance at 801x567: question00 opened in the main
  conversation; progress changed25->75 with multiline output; a further poll
  preserved expanded environment details and offset379. Failure folded the
  exact submission and exposed exit1 on expansion. Success did not interrupt
  historical reading; returning latest opened the exact eligible Run's results.
  Immediate Escape retained details and persisted dismissal; the next Escape
  closed the panel. Cleanup markers removed pending review and folded the
  completed process. The Chinese unsent draft survived all these transitions.
- During QA the isolated host exited for an undetermined reason. Its process
  absence and refused loopback connection were verified before restarting only
  that host. The native client recovered history/draft automatically. No cause
  is claimed from the empty recovery stderr log; production Wisp was untouched.
- Fixed standalone historical terminal-card dismissal consulting the latest
  page instead of the visible one. Added cleanup/deferred-prompt/dismissal scope
  tests. Full C# harness, QA/restored-source publishes, fmt/diff and native
  design/settings synchronization passed. Publish retains cached NU1900. Latest
  source has no temporary startup fixture. Live evidence:
  `test-results/winui-historical-runs-live-evidence.json`; latest QA output:
  `target/native-windows-parity-history-run-verified`; source output:
  `target/native-windows-parity-history-run-source`. The dismissal-only final
  change was model-tested/rebuilt, not separately exercised in a live control.
- The results browser correctly could not list a real server workspace for the
  synthetic local-context Run. Real remote browse/harvest/cleanup, live uncertain
  cancellation and multi-session slow-read journeys remain outside this batch's
  acceptance. The required full stable-source regressions and packaged same-DPI
  comparison remain open, as do assistant/publication/MCP/source-quoting and
  remaining command routes. This batch does not complete the overall goal.
- Publication implementation: added scoped shared mutation/source contracts,
  native item/revision drafts, paged file/Run/message selection and exact manual
  source locators, evidence state/snapshots/lineage/reviews, readiness findings and
  waiver recording, two-step freeze, child revision cloning, reproduction reports
  and Capsule export. Selection choices now match the backend's `rejected` state.
  Message locators preserve UTF-8 boundaries and the persisted content digest.
  Source selection and binding are separate steps with topmost Escape handling;
  bounded, wrapping message cards identify sequence and excerpt.
- Publication real-window checks at 801x567: created the paper and multiline
  Chinese/emoji figure item, bound the exact CSV version and changed it to
  rejected. Selecting the single character `合` from persisted message70 bound
  byte range0..3 with its exact digest and a one-character snapshot. This was
  read back from the isolated backend, not inferred from the rendered selection.
- A missing figure-evidence finding blocked freezing. Recording a synthetic-QA
  waiver invalidated the check; rechecking enabled confirmation. The first click
  and immediate Escape kept the stored revision draft with no manifest; explicit
  confirmation after a fresh check froze it. A native-created child revision
  retained the parent identity while the original remained frozen. Readiness
  explanations now remain visible before checking, fixing a live-discovered
  disappearance after save; the corrected view was verified in a new window.
- The system directory picker opened, and Escape cancelled it while retaining
  the Publication page. The editable path route exported a private Capsule from
  the frozen fixture. Its SHA256 matched the backend build record and all ten
  listed archive-entry checksums passed; the manifest retained the exact message
  excerpt, synthetic limitation and excluded-file selection semantics. This
  archive is an acceptance fixture, not evidence of scientific reproducibility.
- The original isolated host exited with no diagnostic cause established.
  Process absence and refused loopback were verified. The client retained its
  destination and blocked writes without replay. After restarting only that host,
  readback showed no build record or file. UI refresh plus explicit reconciliation
  acknowledgement enabled a new user action, which created exactly one build.
  Production Wisp remained untouched. New-item lost-reply regressions cover both
  no committed row and committed-before-lost-reply, retaining draft IDs and local
  corrections without resurrecting a duplicate unnamed draft. Invalid item
  fields now display feedback without dispatching a mutation.
- Publication validation: Rust Publication28 and DTO84 passed, WASM passed with
  four existing Leptos warnings. The full C# harness passed again after the
  recovery fixes; QA and restored-source WinUI publishes passed (cached NU1900).
  Format/diff and native design/settings synchronization passed. Live evidence:
  `test-results/winui-publication-live-evidence.json`. Remaining acceptance:
  actual directory selection, precise manual source kinds, live reproduction
  with eligible source Run, per-revision draft journey, policy-change journeys,
  packaged same-DPI comparison and stable-source full regression. Research
  Assistant, MCP Apps, Office/scientific quoting and remaining commands are
  still open; this batch does not complete the overall request.
- Office/scientific quoting implementation now carries DOCX preview pages,
  PPTX slide numbers, XLSX sheet/cell/formula metadata and scientific source-text
  line ranges into session-owned composer quotes. Native panels own the source
  path and scope; source drafts are labelled when unsaved. Scientific canvas
  atom/residue selection is not implemented. The C# contract harness passed for
  this batch, but its browser selection tests, WinUI build and real-window
  acceptance remain pending.
- Submission checkpoint: the user requested a PR containing the current changes
  and explicitly stopped further local testing. No additional local test/build
  run is part of PR preparation. Earlier focused results apply to their recorded
  batches, not to the complete final snapshot. Full Research Assistant, MCP Apps,
  remaining command routes, outstanding live acceptance and stable-source full
  regression remain follow-up work. Temporary QA outputs are excluded from Git.
