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

Conversation controls and search remain incomplete:
remaining global composer helpers and session
transfer/import/export and project-file search still need implementation and
verification. Later outcome groups remain open.
