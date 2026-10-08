# macOS workbench completion

The user resumed implementation on 2026-10-08 with four required outcomes:
research assistant, publication evidence editing, specialized viewers, and a
workflow canvas. The historical pause in the 2026-10-06 parity plan does not
apply to this work. Implement each group in reviewable increments; the overall
goal remains open until all four groups pass their acceptance checks.

## Outcomes and acceptance

1. Publication: create papers and revisions; edit the outline; select exact
   evidence sources and message excerpts; change evidence selection/visibility;
   inspect readiness, lineage, drift and reproduction; check, waive, freeze,
   verify and export capsules through the existing backend. Enforce revision
   ownership, immutable frozen revisions, and unknown-write recovery. Preserve
   drafts and discard late replies after navigation. Test immediate Escape on
   nested editors without moving focus first.
2. Assistant: open the existing assistant conversation with its restricted tool
   set; select visible project context; show its plan and calendar; manage
   persistent automations and conversation timers through the shared scheduler.
   Privacy filtering must occur before project reads. Use fake transports in tests.
3. Viewers: complete HTML, PDF/Office and scientific sequence/structure viewers
   beside the existing Markdown, table and image previews. Include corresponding
   WebView source inspection, navigation and quoting interactions where supported.
   Keep large data bounded and validate file/project ownership.
4. Workflows: render and edit stages and dependencies on a canvas; support node
   selection, pan/zoom, validation, save/copy and conversion. Reuse execution and
   approval paths rather than implementing a second runner.

## Verification

Run focused Swift and Rust tests per increment, render native layouts at normal
and narrow widths, and inspect English/Chinese and light/dark variants. Before
overall completion run the repository gates (`cargo fmt --all -- --check`,
`cargo test --workspace`, UI wasm check, Playwright), the native Swift suite,
contract/design sync checks, and paired current packaged-app smoke checks. No
real credentials, network, cluster, or SSH host is required in automated tests.

## Progress

- Started from `5aec1c29` on `codex/macos-workbench-completion`, clean checkout.
- Publication backend already exposes typed source and mutation commands. The
  first increment connects the native editor to these existing commands.

- Publication editing is implemented: exact UTF-8 message spans, scoped drafts
  and unknown-write guards, check/freeze, clone, waiver, reproduction and ZIP
  export. The isolated packaged app saved a Chinese/emoji outline, selected an
  exact 0–19-byte excerpt, froze v1, exported a checksum-verified capsule and
  cloned v2 as an editable draft. Native tests: 575 passed including rendered
  variants; DTO and backend focused tests: 3 each. WebView: 1023 passed, 2
  skipped, one startup timeout passed on isolated rerun. Workspace Rust unit
  tests passed; the first doc-test run encountered dependency artifact drift
  during a concurrent QA build, and the isolated doc test passed. A serialized
  full rerun is in progress. Reproduction execution uses fake runners in tests;
  the manual message-only fixture correctly offers no source run to reproduce.

- Assistant increment: explicit native assistant DTO/adapter, restricted
  conversation, privacy-filtered project context/plan, shared calendar, daily
  recap and persistent automation management, and per-conversation timer panels.
  Unknown writes require fresh reads and explicit acknowledgement. Focused
  tests: 9 native tests (including rendering), one shared DTO fixture test, one
  backend assistant/privacy test, and three native calendar tests passed. The
  prior full Swift suite passed 581 tests; final suite includes the new timer
  tests. Packaged smoke is scheduled with the remaining viewer/canvas increments.
  Serialized Rust rerun passed all executed tests but stopped when a generated
  wisp-relay test executable was missing; final validation must rebuild that
  artifact and rerun the suite before overall completion.

- Viewer increment: native PDFKit pages/search/zoom and bounded page quotations;
  shared offline Office, HTML, sequence/alignment, structure and molecule
  renderers in disposable WebKit surfaces. Real WebKit tests passed for DOCX,
  XLSX, PPTX, FASTA, Clustal, PDB, RDKit/WASM and opaque HTML selection. Scoped
  image preparation, remote PDF byte limits and stale quotation rejection are
  tested. The build routing tests now verify renderer assets and clean rebuilds.
- Workflow increment: native stage/dependency canvas, selection, node dragging,
  pan/zoom/fit, full metadata editing, cycle checks, rename propagation,
  built-in copies and conversion-source preservation. Save/delete uncertainty
  requires a fresh read and acknowledgement. Five focused tests and the shared
  native/backend template fixture passed, including immediate nested Escape
  and normal/narrow English/Chinese light/dark rendering.
- Final acceptance completed: all four outcomes are connected and verified.
  The final Swift run passed 581 UI plus 19 core tests without skips, including
  eight real WebKit renderer kinds, normal/narrow locale/theme renders and
  zoom-independent node motion. Rust workspace passed 2679 tests including
  doc tests. UI wasm, design/contract sync, formatting and six fake-compiler
  build routing tests passed. Playwright passed 1021 with two skips and three
  pre-behavior startup/wait timeouts; all three passed isolated serial reruns.
- Paired current 1.18.0 packages passed strict code-signature verification and
  real UI smoke on the same isolated fixture. Native assistant and timer writes
  appeared in WebView as paused schedules. Publication frozen v1/draft v2 and
  the renamed three-node/two-stage workflow read correctly in both clients.
  Native PDF page search/quote, HTML/source/quote, XLSX formula/cell quote and
  packaged RDKit rendering passed. Immediate Escape retained each parent layer.
- Packaged smoke found and repaired node drag double-scaling and missing Office
  layout activation. After rebuilding, a 120×80-pixel drag at 69% moved the node
  exactly 120×80 pixels, pan worked, and the spreadsheet grid was visible.
  The malformed historical zero-seq toolbar fixture was retained for its
  existing evidence; valid empty-session data was used for timer acceptance.
- Differences and limits are recorded in
  [the paired comparison](../../macos-workbench-comparison-2026-10-08.md).
  No real model/SSH/cluster/reproduction job was run; those paths use fake
  transports/runners in automated tests. The message-only evidence fixture has
  no source run to reproduce. The four-module completion does not claim full
  product parity; Hooks and platform-specific layout differences remain.
