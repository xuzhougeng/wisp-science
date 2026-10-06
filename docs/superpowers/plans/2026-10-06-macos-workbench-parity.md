# macOS SwiftUI workbench parity

Baseline: `c296e134`, 2026-10-06. The user requested the complete sequence:
conversation controls and search → files and scientific viewers → research
assistant and timers → publication and workflow editing.

This is an active implementation ledger. A completed increment does not complete
the objective. Existing Rust commands and `wisp-dto` contracts are the behavior
source; SwiftUI must preserve project/session ownership, unknown-result handling,
drafts, privacy filtering and the window Escape stack.

## Required outcome

- [ ] Conversation controls: built-in and ACP mode/configuration, Plan decisions,
  Fast, scoped approval, completion/delegation/specialist options, context view,
  manual compaction, rewind/undo, message editing and branches.
- [ ] Session management: branch navigation, cross-project copy/move, export,
  archive import and Codex/Claude import.
- [ ] Search: persisted cross-project projects, sessions and artifacts, project
  files, commands, keyboard selection and exact-result navigation. Search must
  find older conversations independently of the five recent home entries.
- [ ] Files: local and remote location selection, upload/drop, sorting, selection,
  batch actions, text editing and downloads with scope and conflict checks.
- [ ] Viewers: Markdown, HTML, CSV/TSV, images, PDF, Office, FASTA/MSA and molecular
  structures with the corresponding WebView interactions and source quotes.
- [ ] Research assistant: project context, assistant conversation and calendar,
  automation management and session timers with persisted state and live status.
- [ ] Publication: create/edit publications and revisions, structure/evidence
  editing, source inspection, checks, reproduction and freezing.
- [ ] Research journey: relationships, manual records, recap editing and run
  navigation in addition to the existing daily view.
- [ ] Workflow: readable stages/dependencies and visual node/edge editing,
  validation, save/copy, conversion and existing execution/approval controls.

## Verification

Each increment needs contract/model tests for successful, unavailable, stale and
failed operations; meaningful rendered/native interaction tests including
immediate Escape. Use synthetic fixtures and fake services without credentials,
real remote machines or network-dependent tests. Record actual results below.

Required repository gates: `cargo fmt --all -- --check`, `cargo test --workspace`,
`cd ui && cargo check --target wasm32-unknown-unknown`,
`cd ui-tests && npm ci && npx playwright test`, and the macOS Swift suite.
MCP changes additionally require `cargo run -p wisp-mcp --example smoke`.
Before completing the goal, compare both UIs on the same fixture in light/dark,
Chinese/English and normal/narrow windows, and verify the entire required outcome.

## Implementation evidence

### Increment 1: modes, history branches and persisted search

- Implemented built-in Plan/Fast controls using the existing native commands.
  Unconfirmed mode changes prevent sending until an authoritative snapshot is
  read; drafts and the selected project/session remain intact.
- Implemented exact-turn history branching, edited branch drafts and transcript
  rewind. Stable sequence/digest and the inspected revision guard mutations;
  unknown results are never replayed and require explicit reconciliation.
- Replaced recent-title-only search with persisted cross-project project,
  conversation-body and artifact search. Added scoped commands and exact artifact
  navigation. Query changes and closed sheets discard late responses.
- Added contract/model/navigation tests and immediate-Escape tests. The full
  Swift suite passed 394 tests; subsequent targeted checks cover the additional
  uncertainty-on-reopen case and rendered composer updates. Light/dark and
  419-point-wide fixtures were inspected.
- Native design resource check, Rust formatting and wasm check passed. The
  workspace Rust suite exposed eight macOS artifact-operation failures from
  workspace-root aliases; the defect also reproduced in an isolated test and is
  being repaired separately. The full Playwright suite is running and its five
  failures require isolated reruns.

### Increment 2: ACP settings and Plan decisions

- Decoded the shared ACP session state, including grouped select choices and
  boolean configuration. The interface uses advertised IDs, hides a duplicate
  mode selector when configuration owns it, and bounds expanded configuration
  height. Unbound, running and read-only states cannot write.
- Rendered structured native/ACP plan proposals with Markdown, status and
  priority. Approve confirms the exact exit mode by reading a fresh snapshot
  before dispatch. Save exits without sending. Stale proposals, changed input,
  unknown mode/send results and navigation never trigger a replay.
- Added successful/failed/stale/read-only/duplicate-write tests. Rendered both
  native and ACP plans in Chinese/English, light/dark at 419 points and inspected
  representative results. Targeted checks passed; the subsequent full Swift
  suite passed with no failures.

### Increment 3: scoped session options

- Added session full permission with explicit nested confirmation, delegation,
  inline/background completion, automatic resume/review and pre-conversation
  specialist selection. Responses must confirm both ownership and the requested
  value; wrong-owner, unknown and unconfirmed writes cannot be replayed.
- Options reads can be cancelled; saves retain the sheet. Immediate Escape tests
  caught and fixed a read/save dismissal mix-up. A permission confirmation closes
  before options without moving focus or writing.
- Targeted model/contract/native interaction checks passed. English dark options
  and permission confirmation were rendered and inspected. The full Swift suite
  passed 408 tests after the final render adjustment, with no failures.

### Prerequisite repair: artifact workspace aliases

- The full workspace gate reproduced eight existing macOS artifact-operation
  failures. Absolute stored references used `/var`, while validation used the
  physical `/private/var` root, so preview incorrectly retained eligible files.
- Added registered-root suffix resolution before the existing physical-path
  validation, including transfer record rewriting. Inner links and external
  paths remain rejected. Synthetic recovery journals now use the physical root
  as production does.
- All 12 artifact-operation tests passed, including a new workspace-alias test
  and existing move/delete, collision, shared-snapshot and recovery tests.
  Formatting passed; the full workspace gate passed after this repair. New
  context/compaction changes subsequently require their own regression gate.

### Increment 4: file-aware turn undo

- Added the shared turn-undo preview to the latest completed built-in response.
  Restoration/deletion, artifact records, unsupported files and conflicts are
  listed before confirmation. Conflicts prevent confirmation; the existing host
  rechecks files when applying the exact turn/revision.
- Preserved drafts, global turn identities, read-only/ACP/running/queue guards
  and uncertain-write handling. Preview reads never lock the conversation or
  become uncertain mutations; closing or navigating discards late replies.
- Thirteen targeted control checks passed, including six new undo contract,
  model, immediate-Escape and render checks. Chinese/English, light/dark at
  419 points were rendered and representative screenshots inspected.
- The full Swift suite passed 414 tests after this increment, without failures.
- The WebView gate's three transient failures passed isolated reruns. The two
  reproducible tutorial failures were stale expectations after baseline's new
  map_items article; updated directory/navigation expectations passed all 29
  tutorial tests. This test-only prerequisite is committed separately.

### Increment 5: model context and compaction

- Added an optional `context_view` capability, shared DTOs and an owner-checked
  native read of the persisted head working set, context details and compaction
  metadata. Older hosts and ACP sessions hide this built-in-only entry.
- Added regular/semantic compaction through the existing turn request ledger.
  The operation preserves staged drafts, attachments and references; it never
  turns those inputs into a compaction prompt. Unknown acknowledgements reconcile
  only through the exact request ID and are never replayed.
- Added compaction cards, checkpoint inspection, latest-epoch undo and nested
  confirmations. The shared undo command accepts an optional inspected epoch
  and compares it under the existing workflow lock before changing the store.
  The response explicitly identifies the removed epoch rather than the parent.
- Nine targeted Swift tests passed. Shared context DTO and native projection
  checks passed; the new stale-epoch backend test caught an incorrect expectation
  about the existing undo return value. After repair, all five compaction-undo
  backend tests passed. The full Swift suite passed 423 tests without failures.
  Twenty-six WebView compaction/context checks and wasm compilation passed.
  Chinese/English, light/dark context pages, history and semantic confirmation
  were rendered at 419 points and inspected; a cramped picker was corrected.
- The pre-context workspace gate passed 2634 tests, with no failures. Formatting
  and generated native-resource checks passed. The new context changes still
  need the final full workspace run with subsequent conversation increments.

### Increment 6: scoped tool approvals

- Added optional exact-approval scope advertisement and a strict shared scope
  enum defaulting to once for older clients. The host validates scope and grant
  eligibility before atomically consuming the pending request. Special plan,
  workflow, resize and conflict confirmations remain once-only.
- Added native scope selection with explicit scope labels/descriptions and
  existing persisted project/global grants. Selection does not write. Unknown
  replies block duplicate decisions; an explicit fresh read can confirm that
  the exact request is still pending before another human decision.
- Eight targeted native approval checks passed, covering advertised scopes,
  older/read-only/special/stale/malformed requests, response loss, read-only
  reconciliation and immediate Escape. The full Swift suite passed 429 tests
  after the final narrow-card layout adjustment. All six Rust approval regression
  checks and 35 shared native conversation DTO checks passed. The scope
  picker and narrow global-scope card were rendered in both locales/schemes and
  representative screenshots inspected. Generated-resource and formatting
  checks passed; final full regression still remains.

### Increment 7: global composer helpers

- Added a visibly global section for memory, automatic failure analysis and
  reviewer backend using the existing shared commands/contracts. Range edits
  preserve the enabled/other threshold values and reject values outside 1–100.
- Added default HTTP/follow-session/configured chat/ACP reviewer choices. Exact
  media IDs and explicitly configured image profiles are excluded. A pre-save
  reread preserves concurrent persona edits and refuses a changed backend;
  complete specialist fields are retained. Unknown results require a fresh read
  without automatic replay. Closed/cancelled reads and late writes cannot update
  another sheet or dispatch a reviewer write after navigation.
- Fifteen targeted native checks passed (nine new helper tests plus six session
  option tests), covering shared contracts, advertised choices, read-only and
  stale state, unknown/malformed replies, duplicate writes, cancellation and
  immediate Escape. The shared DTO fixture test passed. Options, global controls,
  short-window scrolling and reviewer selection were rendered at 419 points in
  Chinese/English and light/dark; representative screenshots were inspected.
- Full Swift passed 438 tests with no failures. Formatting, wasm compilation and
  generated-resource/command-contract checks passed; final full Rust/WebView
  regression gates are running. The options sheet now scrolls and its nested
  permission confirmation fits narrow windows.

### Increment 8: reviewed session ZIP import

- Added project-sidebar and scoped command-palette entry points using a distinct
  shared archive-import icon. Preview identifies the explicit destination,
  message/artifact counts, bounded message excerpts and existing-import status.
  Confirmation sends the reviewed source session ID and SHA-256 through the
  existing protected native import adapter.
- Added project selection, existing-session inspection and exact-result opening.
  Unknown/malformed/wrong-owner replies disable further imports in that sheet,
  including after a reread or source/destination change. Superseded/cancelled
  reads and closed writes cannot update another view or navigate it. Saving holds
  the sheet; immediate Escape closes its destination picker first.
- Thirteen targeted native checks passed (eight new import tests and five search
  checks). Shared fixtures cover ownership, reviewed arguments/counts, scope,
  duplicate attempts, read-only navigation guards and late replies. Preview,
  result and project picker were rendered at 419 points in Chinese/English and
  light/dark, with representative screenshots inspected. The full Swift suite
  passed 446 tests after final layout/translation refinements, with no failures.
  Formatting, wasm and generated-resource/command-contract checks passed.
  Final full Rust/WebView regression remains in progress.

### Increment 9: Codex and Claude session import

- Added project-sidebar and scoped command-palette entry points with a distinct
  shared conversation-import icon. Uses existing project/provider/environment
  source/list/preview/result contracts with strict ownership, source identity,
  bounded excerpts, state, count and hash validation.
- Added cached listing/rescan, text filtering, 25-row pages, single preview/import
  and a batch across all filtered pages. Each batch write reads a fresh preview;
  independent source-read failures do not stop other candidates. Stop completes
  the current write or prevents a write after an in-flight preview. Unconfirmed
  writes halt the batch without replay and remain blocked after refresh or
  changing destination/provider/environment. Exact confirmed results can open
  their target conversation; source drafts are retained by normal navigation.
- Initial nine targeted native checks passed, including shared fixtures,
  scope/advertised choices, pagination, duplicate writes, late/closed responses,
  batch progress and immediate Escape for sources and previews. Cancellation
  and stop-during-preview coverage was subsequently added; all 15 import/search
  targeted checks passed. The full Swift suite passed 456 tests without failures.
  List, results, previews and source pickers were rendered at 419 points in both
  locales/schemes and representative screenshots inspected. Formatting, wasm
  and generated-resource/command-contract checks passed.

### Latest repository regression

- The full context/approval Rust workspace gate passed 2639 tests across 45
  suites, with no failures or ignored tests. The subsequently added helper DTO
  fixture also passed its targeted check.
- The full WebView Playwright gate passed 1023 tests, with two existing optional
  real-service tests skipped and no failures. `npm ci` succeeded before the run.
- These gates establish the completed increments' current baseline. They do not
  complete the remaining outcome groups or packaged/native acceptance.

### Increment 10: reviewed cross-project conversation transfer

- Added conversation-menu and scoped Cmd+K copy/move entry points with distinct
  shared icons and explicit destination selection. The native DTO/fixture binds
  source, destination, mode, all-epoch message count, transcript revision, file
  choice and confirmed new frame ID. The shared transfer command rechecks a
  native preview's revision while holding its workflow/agent lock and rejects
  unconsumed queued turns. Its optional revision argument keeps WebView callers
  compatible.
- Move previews reuse the existing artifact plan and fingerprint. Lists identify
  movable artifacts/files and retained shared/uploaded/changed/unavailable
  entries; an unavailable file preview still permits transcript-only movement.
  Files default off and reset after rereading or changing destination. Existing
  file collision, stale fingerprint, recoverable file operation, archive,
  exploration and branch restrictions remain authoritative in the host.
- Confirmed moves retain the text draft at the exact new frame without sending;
  staged attachments/references must be cleared because they belong to the
  source project. Copy retains the original draft. Lost/malformed/wrong-scope
  replies block further submissions in that sheet even after rereading or
  changing destination; a persistent explanation remains after rereading. Closed,
  cancelled and superseded reads or writes cannot navigate a different view.
  Saving holds the sheet and immediate Escape closes its project picker first.
- Retained artifacts or Runs can keep a deleted source frame as a lineage
  tombstone. Added a separate live-frame lookup for conversation operations,
  preserving raw ownership for lineage. Native reads/mutations, shared transfers
  and both built-in/ACP turn paths reject stale source frames under their locks.
  Regression checks cover moved conversations with retained artifacts and
  deletion with/without retained Run lineage.
- Sixteen targeted native checks passed (eleven new transfer tests and five
  search checks). Two host checks cover source ownership/title, all context
  epochs, saved events, stale revisions and queue exclusion; the shared DTO
  fixture test passed. Seven WebView session-action/file-operation checks passed
  after `npm ci`. Copy, move, retained files, file-preview failure, confirmed
  results and uncertainty were rendered at 419 points in both locales/schemes;
  representative final screenshots were inspected. Full Swift passed 467 tests
  after final layout and fresh-preview title refinements. Formatting, wasm and
  both generated resource/command-contract checks passed. The initial full Rust
  gate passed 2643 tests; three new live-frame regression tests passed separately
  and the full gate with those guards is running. Full Playwright passed 1022
  tests with two existing optional-service skips and one homepage-loading
  timeout before the activity-indicator assertions. All three indicator checks
  passed on isolated rerun; no product or test changes were needed for that timeout.

Conversation controls and search remain incomplete. The Files panel already
searches project directories, matching the WebView's separate Files search.
Cmd+K reference/new-window actions are tracked separately. Session export is tracked below. Later outcome
groups remain open.

### Increment 11: conversation relationship navigation

- Added optional persisted source, branch-state and subagent-parent metadata to
  the shared project-browser DTO. Project reads use existing store queries;
  global five-recent reads retain their previous shape, and unavailable metadata
  in an older read-only database does not invent a relationship. Legacy links
  without a checkpoint keep the shared WebView behavior. Orphaned checkpoints
  advertise their state without an active source or sibling relationship, and
  the dialog explains that their saved records remain readable.
- Added a conversation-menu and scoped Cmd+K relationship dialog, identifying
  source, sibling branches, direct branches and subagents. Sidebar icons and
  accessible labels identify states. Only advertised records in the same project
  can open; missing/deleted/foreign/self references cannot create navigation.
  Opening uses normal conversation navigation and retains unsent drafts.
- Ten targeted native checks passed (five new relationship checks and five
  search checks), covering scoped navigation/drafts, missing/deleted/foreign/self
  links, duplicate rows, invalidated checkpoints and immediate Escape. The final
  full Swift suite passed 472 tests with no failures. Twelve relationship layouts
  were rendered at 419 points in both locales/schemes, including invalidated
  sources, and representative final screenshots were inspected.
- All eleven project-query tests passed, including persisted active/merged/
  orphaned states, subagent parents and old read-only database compatibility.
  The shared DTO fixture and all ninety DTO tests passed. Formatting, wasm and
  both generated-resource/command-contract checks passed. Full repository gates
  are still tracked separately; later outcome groups remain open.

### Increment 12: reviewed conversation ZIP export

- Added a shared DTO/fixture and owner-checked native host preview/write adapters.
  Extracted the established WebView archive preparation/writing boundary so both
  clients use the same messages, transcript, tool, terminal-event and optional
  artifact/provenance entries. Export explicitly captures the current model
  context, excluding pre-compaction history; it does not claim a lossless backup.
- Added conversation-menu and scoped Cmd+K entries with a distinct shared icon,
  preview counts/paths/sizes, artifact inclusion, native save panel, saved path/
  checksum and explicit Finder navigation. Cancelled save panels write nothing.
  Idle archived/read-only conversations are eligible; scope, running/stopping,
  approvals, review and queued messages guard export. Closed/superseded reads and
  late writes cannot affect another view. Unknown replies retain the attempted
  save path and cannot replay, including after a fresh preview/file-choice change.
- Native preview revisions bind the saved source and exact file bytes. The
  shared writer stages beside the destination and preserves a previous archive
  on failures; size changes are rejected and native writes also verify hashes.
  Includes a macOS workspace-alias repair discovered by the actual import
  compatibility test: relative manifest paths survive `/var`/`/private/var`.
  Included source files and hardlink aliases cannot be overwritten by export.
- Added cancellation-after-dispatch protection and regression checks to Copy/
  Move and ZIP import, preventing a cancelled mutation's acknowledgement from
  enabling duplicate writes. Added shared export contracts, native scope,
  success, unavailable/invalid/stale/closed/cancelled responses, file-choice,
  save-panel cancellation, immediate Escape and locale/scheme renders. Backend
  checks cover revisions, size/content changes, destination preservation, source
  alias protection, compaction scope and export/import artifact restoration.
- The earlier full Rust gate with live-frame tombstone guards finished: 2646
  tests across 45 suites passed, without failures. It predates the export code
  and is not used as proof of this increment. All nine export/backend compatibility
  checks and the shared DTO fixture passed after the alias repair. Thirty-seven
  targeted Swift checks passed; the final full Swift suite passed 485 tests with
  no failures, including the final save-path and locale refinements. Sixteen
  export layouts were rendered at 419 points in both locales/schemes; representative
  previews, saved results and uncertainty layouts were inspected. Formatting,
  wasm and both generated-resource/command-contract checks passed. New full
  Rust/WebView regression gates remain in progress; final packaged/native
  acceptance and the remaining outcome groups are still open.

### Increment 13: scoped search references

- Added Shift+Enter and an explicit result-context-menu reference action. The
  existing scoped composer catalog rereads privacy/eligibility and supplies the
  exact ID/kind and fresh label before the existing composer stages/deduplicates
  the chip. No send, navigation or draft replacement is involved. Long Unicode
  titles are bounded at scalar boundaries for the existing 512-byte query limit.
- Closed, cancelled, superseded and changed-scope reads cannot stage into a
  different draft; unavailable/hidden/missing/wrong-owner results remain errors.
  Search-result navigation also rechecks its origin at Task startup. Enter and
  Shift+Enter use native field-editor commands while preserving IME candidates.
  Made the search sheet responsive and translated its shortcut/close controls.
- Seventeen targeted Swift checks passed, including six new reference tests,
  actual conversation staging/deduplication, field-editor/IME callbacks and the
  existing search/composer checks. Four locale/scheme layouts were rendered at
  419 points and representative final layouts were inspected. The final full
  Swift suite passed 492 tests with no failures after the layout/translation/
  matching-query refinements. Formatting and both generated-resource/command
  checks passed. No new Rust command or schema is required. Independent windows,
  composer focus after staging and later outcome groups remain open.

### Increment 14: composer focus after search references

- A successfully staged reference requests focus for the exact existing composer
  after the search sheet dismisses. The request is consumed once; it never activates
  a window, changes text/caret or sends. It binds the database, project, session,
  navigation generation, search generation, editor and window identities.
- Changed navigation (including leaving and returning), reopening search, settings,
  another key window/sheet, detached/reparented/hidden/read-only editors and active
  IME composition discard the request. Ordinary search cancellation/navigation and
  failed reference reads do not request focus.
- All 33 targeted Swift checks passed, including five new AppKit/editor and actual
  conversation/browser focus checks. The final full Swift suite passed 497 tests
  without failures. Formatting and both generated-resource/command checks passed.
  This changes focus only; the existing locale/scheme search and composer renders
  were rerun. The completed export WebView full gate passed 1022
  tests, with two existing optional-service skips and one homepage-loading timeout
  before the activity-indicator assertions. All three activity-indicator tests
  passed on isolated rerun; the full-run failure evidence is retained in `/tmp`.
  No product or test changes were made for that timeout. The completed export Rust
  workspace gate passed 2655 tests across 45 test summaries, without failures or
  ignored tests. The focus increment adds no Rust command/schema. Independent
  windows and later outcome groups remain open.

### Increment 15: independent workspace windows from search

- Added value-based SwiftUI workspace scenes with a fresh identity per open. Each
  scene owns its browser/conversation/navigation/draft/panel models and shares a
  database-keyed descriptor transport registry. A window changing database uses
  that database's host; same-database windows retain a shared transport. The request contains no draft
  or token. Project/session targets are reread; unavailable/hidden/missing/foreign
  targets fail visibly without recent-session fallback, and cancelled reads cannot
  make the window ready. The primary Dock/Finder reopen behavior is retained.
- Added Cmd/Ctrl+Enter and project/session result context actions, Cmd+N and a scoped
  `>new window` action. Artifacts keep WebView's original-window preview. Shift+Enter
  preserves reference priority and IME commands stay with AppKit. Application menu
  actions use the focused scene's model; initialization failures cannot receive
  workspace actions. The translated shortcut legend fits narrow windows.
- Thirty-six targeted Swift checks passed, including six new scene identity,
  database/ownership/failure/cancellation and real browser/conversation isolation
  checks using fake queries and a shared fake host. The final full Swift suite
  passed 503 tests without failures after the database registry refinement.
  Chinese/English light/dark search layouts were
  rerendered at 419 points and representative results inspected. Formatting and
  both generated-resource/command checks passed. No Rust command or schema changed;
  the completed 2655-test Rust gate and WebView full gate/rerun are recorded above.
- Files/viewers, assistant/automation/timers, journey/publication/workflow editing
  and final packaged multi-window acceptance remain open. These conversation and
  search increments do not complete the overall goal.
