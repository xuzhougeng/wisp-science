# macOS SwiftUI UI analysis (2026-10-19)

Scope: the native SwiftUI client in `apps/macos`, measured against the Leptos
WebView client in `ui/` that it mirrors. Read-only analysis of the tree at
`94f9a0be`; no tests were run.

## 1. Where the native app stands

The SwiftUI app is not a prototype. It is a second full client over the same
Rust host, with its own transport, contract fixtures, design-token export,
render tests and signed DMG in the macOS release workflow.

| Measure | SwiftUI (`apps/macos`) | WebView (`ui/`) |
| --- | --- | --- |
| Source lines | 20,890 Swift in 157 files | 96,202 Rust in 102 files + 13,116 CSS |
| Largest files | `NativeConversationModel.swift` 820, `ProjectWorkspace.swift` 655, `ProjectBrowserModel.swift` 629 | `main.rs` 18,981, `settings_view.rs` 8,565, `i18n.rs` 7,918 |
| Components | ~125 view/model files | 143 `#[component]` functions |
| Tests | 615 XCTest functions (12,997 lines), incl. opt-in WebKit renders | 68 Playwright spec files |
| Top-level overlays | sheets per feature, window Escape stack | 97 root `RwSignal<bool>` toggles in `main.rs` |

Parity work landed in three bursts: the 2026-09-25 reliability pass (PR #1361),
the 2026-10-06 workbench ledger (18 increments), and the 2026-10-08 four-module
completion plus polish (PR #1480 and `codex/macos-parity-polish`). The ledger in
`docs/superpowers/plans/2026-10-06-macos-workbench-parity.md` still has four of
nine outcome groups unchecked (conversation controls, session management,
search, files), even though many of their increments are implemented.

### Drift since the last native burst

Commits touching `ui/src` since 2026-09-20, classified by what they did under
`apps/macos`:

| Class | Count | Meaning |
| --- | --- | --- |
| Swift sources changed | 4 | real native work |
| only `Resources/native-english.json` | 48 | the design-sync script re-exported strings; no native behavior |
| nothing under `apps/macos` | 70 | WebView-only features or fixes |

The "resources only" class is the dangerous one: `git log` makes those commits
look native-aware, but for example #1547 (per-session Runs tab) and #1567
(project memos tab and `/memo` picker) only added English strings. The native
right pane still enumerates `artifacts, agents, notebook, highlights, files,
provenance, hosts, sidechat` (`apps/macos/Sources/WispProjectBrowser/NativePanelTabs.swift:7`)
while the WebView `RightTab` enum also has `Runs` and `Memos`.

## 2. How the native UI is built

### 2.1 Process topology

```
Wisp Science SwiftUI.app
├── WispScience (SwiftPM executable, SwiftUI shell)
├── wisp-service            one short-lived process per read query (JSONL over stdin/stdout)
└── wisp-tauri --native-settings-host
        loopback HTTP /invoke + bearer token, descriptor beside wisp.sqlite
        bundles the real Trunk frontend so the regular app can adopt this host
```

- `ProjectBrowserClient` (`apps/macos/Sources/WispProjectBrowser/ProjectBrowserClient.swift`)
  spawns `wisp-service` per `list_projects`/`list_sessions`/`get_transcript`
  call with a 30 s deadline. Reads are read-only SQLite; only project stars use
  `--allow-project-writes`.
- `NativeSettingsClient` (`NativeSettingsClient.swift`) is an actor that
  discovers or launches the full desktop host, validates the descriptor
  (loopback, 64-hex token, same database path) and POSTs
  `{schema,id,command,args,project_id}`. Every other native feature rides on
  this one `invoke` call: conversations, files, panels, settings, publication,
  journey, assistant, library, calendar, search.
- Contracts live in `crates/wisp-dto` (`native_conversations.rs` alone is 1,734
  lines) with JSON fixtures under `contracts/*/v1` decoded by Rust, Swift and C#.
  This is the strongest part of the design: Swift never mirrors a struct by hand
  without a fixture test.

### 2.2 Conversation data flow

`NativeConversationModel` polls `native_conversation_snapshot` every 350 ms
while running, 1.5 s idle, 2 s after an error
(`NativeConversationModel.swift:382`). Snapshots replace the visible page; there
is no token-delta stream. Send returns acceptance with a UUID `request_id`; the
model never retries a mutation and surfaces "uncertain" states that require an
explicit acknowledgement button. The same rule is applied uniformly to pin,
rename, delete, transfer, export, enqueue, approvals, history mutations and
settings saves.

Consequences for the UI:

- Live text appears in 350 ms steps rather than per token. The WebView has a
  throttled Markdown prefix plus an immediate plain-text tail; the native view
  re-renders the whole `visibleItems` array on each snapshot.
- `ForEach(Array(conversation.visibleItems.enumerated()), id: \.offset)`
  (`NativeConversationView.swift:50`) identifies rows by position. There are 77
  `id: \.offset` loops in the UI target. Positional identity defeats SwiftUI
  diffing, so an inserted older page or a folded tool row re-creates every
  row below it. This is the likely root of the "35-turn fixture" performance
  notes in the parity progress doc.

### 2.3 Presentation layer

- Transcript blocks are one selectable AppKit text document
  (`NativeSelectableMessage.swift`) with block Markdown, native tables, SwiftMath
  LaTeX, highlight.js grammar for code and inline images. Cross-block selection,
  quote and highlight work because it is one `NSTextView`, not a `VStack` of
  `Text`.
- Specialized documents (PDF via PDFKit; DOCX/XLSX/PPTX, HTML, FASTA, Clustal,
  PDB, SMILES) render in a disposable `WKWebView` that serves bundled vendor
  modules (`native-rich-preview.html`, `vendor-runtime/`). This is the same
  offline renderer set as the WebView, so scientific fidelity is shared.
- Terminal is SwiftTerm 1.19 (`NativeTerminalModel.swift`).
- Layout is hand-rolled `HStack`/`VStack` with a 248 pt sidebar, a draggable
  280–600 pt inspector and a 220–600 pt terminal dock
  (`ProjectWorkspace.swift:46-254`). There is no `NavigationSplitView`,
  `NavigationStack`, `.sheet`, `.popover` or `.alert` in the UI target by count
  of the modifier spelled with a leading dot; sheets are attached via
  `.sheet(isPresented:)` chains on `ProjectWorkspace` (13 of them on one view).

### 2.4 Design tokens and icons

`scripts/sync_native_design.py` exports `palette.json` (every WebView palette
in light/dark), 65 `icon-*.svg` files from `compose_icon()`, both wordmarks,
`settings-navigation.json`, `model-presets.json`, `automation-templates.json`
and `native-english.json` from the Leptos sources. `--check` runs in CI
(`native-preview.yml`) and in `build_native_macos.sh`. `WispDesign.color`
resolves a token through `UserDefaults` (`nativeSettings.<theme>_palette`) on
every call (`WispDesign.swift:34-56`), and `WispDesign.font` reads two
`UserDefaults` keys per call. Neither is cached and neither is observable: a
palette change in Settings does not re-render open views until something else
invalidates them.

### 2.5 Escape stack

`NativeSettingsEscape` (`NativeSettingsControls.swift:202`) is an
`NSViewRepresentable` whose coordinator registers in `NativeEscapeStack.shared`
on mount and removes itself on dismantle; 76 call sites. This matches the
AGENTS.md rule (window-level stack ordered by mount, no `autofocus`, no bubbled
`keydown`). It is a real strength and every new sheet must use it.

### 2.6 Localization

The app is Chinese-first with an English override table: `localized(_:)`
(`WispDesign.swift:151`) looks up the Chinese literal in `native-english.json`
(3,791 keys). The UI target contains roughly 6,300 Chinese string literals but
only 1,111 `localized(` calls, so most labels are not translated when the
locale is English. `docs/native-conversations.md` records this as a known
follow-up ("English preferences still leave Chinese workspace labels"). There
is no `Localizable.strings`, no pluralization, and `localized` reads
`UserDefaults` on every call.

### 2.7 Tests and CI

- 615 XCTest functions across `WispProjectBrowserTests` (transport, DTO
  fixtures) and `WispProjectBrowserUITests` (models, fake hosts, immediate
  Escape, bilingual light/dark narrow renders via `WISP_NATIVE_SNAPSHOT_DIR`,
  opt-in real WebKit renders via `WISP_NATIVE_DOCUMENT_SMOKE=1`).
- `native-preview.yml` runs `swift test` for `apps/macos` and `apps/ios` on
  `macos-latest`, plus the WinUI job. `release-macos.yml` builds, signs and
  notarizes the SwiftUI DMG for both targets alongside the Tauri app.
- Playwright tests cover the WebView only; there is no shared behavioral
  fixture that asserts the same scenario in both clients.

## 3. The WebView UI as the reference

Shapes worth knowing before porting anything:

- `ui/src/main.rs` is one 19k-line `App` component owning 97 boolean overlay
  signals, the Escape stack, pane layout and most event wiring. Feature
  modules (`settings_view.rs`, `agent_workflows.rs`, `chat_render.rs`,
  `publication.rs`, `swarm.rs`, `mandates.rs`, `memos.rs`, `lab_view.rs`,
  `channels_view.rs`, `pet.rs`, `mcp_app.rs`, `acp.rs`, `notebook.rs`,
  `assistant_workspace.rs`, `research_journey.rs`, `trajectory.rs`) are
  components invoked from it.
- `ui/src/i18n.rs` (7.9k lines) is the translation source the native export
  reads from.
- `ui/src/styles/*.css` (13k lines) carry the spacing/radius/type scale that
  `docs/ui-design-principles.md` codifies (`--space-1…7`, three radius tiers,
  `.btn-primary`/`.btn-ghost`, 720 px topbar fold, 960 px inspector drawer,
  1280 px conversation cap).
- Right pane tabs: Artifacts, Agents, Notebook, Highlights, File, Provenance,
  Hosts, SideChat, Runs, Memos.
- Settings: 21 sections in four groups (`settings-navigation.json`), all of
  which the native settings view already connects.

## 4. Parity ledger by surface

Legend: done = connected and accepted in a plan doc; partial = entry exists but
behavior is narrower; missing = no native surface.

| Surface | State | Evidence and gap |
| --- | --- | --- |
| Home (projects, 5 recents, search, calendar, library, assistant, import, new project, docs link) | done | `docs/native-project-browser.md` shell table |
| Project sidebar (new, import ZIP, import Codex/Claude, search, groups, files, journey, publication, library, capabilities, feedback, settings) | done | `ProjectWorkspace.swift:483-654` |
| Sidebar nesting: fold subagent/branch children under a chevron (#1475), drag-reorder groups (#1489), shelved conversations (#1412) | missing | commits were resources-only or none |
| Conversation transcript (block Markdown, tables, LaTeX, images, usage line, plan checklists, compaction cards, ACP cards) | done | `docs/native-conversations.md` |
| Streaming feel | partial | 350 ms snapshot polling, positional row identity |
| Composer (send prefs, IME, attachments, `@ # /` pickers, references, queue, Plan/Fast, options sheet, timer, context view) | done | `NativeMessageInput.swift`, `NativeComposerCompletion.swift` |
| Composer: thinking-effort pill (#1421), unified Agent menu (#1549/#1425), Stop icon yielding to Send (#1419), `/memo` picker (#1567), archived-session `#` search (#1585) | missing | WebView-only commits |
| Queue cut-in / interrupt-and-replace | partial | macOS labels cut-ins read-only; Windows and WebView can act |
| Tool approvals with scope, feedback, Full Permission shortcut (#fcf0ce07) | partial | scope done; shortcut missing |
| Topbar: outline, share, trajectory, archive, inbox, terminal, panel, session menu | done | `ProjectWorkspace.swift:63-136` |
| Topbar More-menu fold at 720 px (#1440), Full transcript / Model view switch | missing | native topbar has no fold or model-view toggle |
| Right pane: Artifacts, Agents, Notebook, Highlights, Files, Provenance, Hosts, SideChat | done | `NativePanelTabs.all` |
| Right pane: Runs tab (#1547), Memos tab (#1567) | missing | strings only |
| Files (locations, sort, select, search, upload/drop, SSH download, text edit, Markdown/CSV reading) | done | increments 16–18 |
| Viewers (PDF, Office, HTML, FASTA/MSA, structures, SMILES) | done | PR #1480 |
| Session management (rename, pin, delete, batch, copy/move, relations, export ZIP, import ZIP, import Codex/Claude, independent windows) | done | increments 8–15 |
| Conversation branches: start exploration, branch merge guidance | partial | branch/rewind done; exploration branches (1 file mentions them) not surfaced |
| Research assistant (three columns, calendar, automations, timers, templates) | done | polish record |
| Research mandates (#1493/#1496) | missing | 0 Swift files mention mandates |
| Agent swarm progress card and Reader retry (#1518/#1573) | missing | 0 Swift files mention swarm |
| Research journey (daily view, manual entries, recap edit, relationships, run details) | done | polish record |
| Research graph page (`show_research_graph`) | missing | 1 Swift mention; no page |
| Publication workspace (papers, revisions, evidence, freeze, capsule, clone, reproduce) | done | PR #1480 |
| Workflow canvas (nodes, ports, pan/zoom, save/copy, conversion) | done | polish record |
| Settings (21 sections) | done | `docs/native-settings.md` |
| Settings: Wisp Lab join/members/knowledge (#1503/#1504), ChatGPT account pool (#1396), decentralized project storage (#1372) | partial | strings synced; verify each editor, several commits are resources-only |
| Pet overlay | missing | settings page exists, no animated pet window |
| MCP Apps embedded in the center pane | missing | documented follow-up |
| Channels (Feishu/WeChat QR) | done | settings |
| Remote access / relay QR | done | settings |
| Window titlebar customization (`window_titlebar.rs`) | n/a | native chrome |
| Onboarding / API-key first run | missing | `show_onboarding` has no native counterpart; docs list "first use" under formal delivery |
| Usage tips on empty conversation (#2c1ad2e1) | missing | native empty state is one sentence |
| iOS (`apps/ios`) | separate product | relay-only remote client with offline demo; not a port of the desktop UI |

## 5. Technical debt and risks in the SwiftUI code

Ordered by how much they slow the next increments.

1. Positional list identity. 77 `id: \.offset` loops. The transcript is the
   worst case because it re-renders on every 350 ms poll. Fix: give
   `ConversationItem` a stable key (host `seq` is already in the snapshot for
   history pages; add it to `items`), then key `ForEach` and `expandedTools`
   on it. This is a `wisp-dto` additive field plus a Swift change, which is
   one PR.
2. God views. `ProjectWorkspace` carries 13 sheets, 20 `@State`/`@AppStorage`
   fields, a `workspaceCommand` switch and the entire sidebar. Every new
   session-scoped sheet lands here. A `WorkspaceSheets` enum with one
   `.sheet(item:)` would cut the modifier chain and make Escape ordering
   explicit. Do this only when the next feature would otherwise add a 14th
   sheet (AGENTS.md: refactor for a concrete reason).
3. Token lookup through `UserDefaults` on every `color()`/`font()`/`localized()`
   call. Cache per theme and publish palette/locale changes through an
   `EnvironmentObject` or `@AppStorage`-backed object so Settings → Appearance
   repaints live. Also required for the planned live font-size application.
4. Localization coverage. 1,111 of ~6,300 Chinese literals pass through
   `localized`. A lint (test that walks `Text("…")` literals containing CJK in
   `WispProjectBrowserUI` and asserts they are wrapped) would stop the gap from
   growing; backfilling is mechanical per file.
5. Polling instead of events. The host already has a UI event writer. A
   long-poll or SSE endpoint on the loopback host (`native_conversation_watch`
   returning when `sequence` advances) would halve idle traffic and make
   streaming text smooth without changing the replace-not-append contract.
6. Resource-only "native" commits. `sync_native_design.py` runs inside
   unrelated PRs, which hides drift. Add a CI check that lists WebView feature
   files changed without a matching Swift change, or at least a
   `docs/native-parity-ledger.md` that every UI PR must touch. Cheap and
   high-signal.
7. `wisp-service` per query. Home refresh forks two processes and parses a
   30 s deadline each time. Not a correctness issue, but the host is already
   running for everything else; moving project/session reads to the loopback
   host would remove a second code path and a second contract.
8. Single `NativeConversationModel` per database. `nativeModels[databaseURL]`
   means switching sessions reuses one model with navigation-generation guards.
   It works, but per-session models (as side chat and terminal already do)
   would remove most of the `guard model.activeSessionID == session` checks
   repeated throughout `ProjectWorkspace`.
9. `runModal()` pickers (13 sites) block the main actor and need the temporary
   Escape listener workaround documented in increment 17. `NSOpenPanel.begin`
   with a completion keeps the stack ordinary.

## 6. Recommended next increments

Each is one PR, one abstraction, with tests, per AGENTS.md.

1. Stable transcript identity (debt 1). Prerequisite for everything below that
   touches the transcript; measurable with the existing 35-turn fixture.
2. Right pane Runs and Memos tabs. Both have shared DTOs
   (`wisp-dto::project_memos`, run cards) and the WebView behavior is fresh.
   Memos is also the only WebView feature with a `/memo` composer command the
   native slash menu lacks.
3. Composer catch-up: thinking-effort pill, Stop-yields-to-Send, unified Agent
   menu, Full Permission shortcut on approval cards, archived-session `#`
   search. All are WebView-only commits from the last month and small.
4. Topbar fold at 720 pt and the Full transcript / Model view switch. Model
   view needs the pruned-tombstone rendering from #1316.
5. Sidebar parity: nested children fold, drag-reorder groups, shelved
   conversations with restore.
6. Parity ledger CI check (debt 6) so the next month does not repeat the
   70-commit gap.
7. Research mandates and swarm progress cards. Larger; both are
   assistant-side features with their own DTOs and should follow the assistant
   model pattern already in `NativeAssistant.swift`.
8. Live theme/font application (debt 3) and the localization lint (debt 4).
9. Onboarding / first-run (API key, model pick) so the DMG is usable without
   first opening the Tauri app.
10. Streaming transport (debt 5) once the transcript identity fix is in.

Deferred by design, not by omission: MCP Apps in the center pane (needs a
sandboxed child WebKit surface like the Windows design), the animated pet,
and any ACP cut-in semantics the host does not offer.

## 7. Questions to settle before starting

- Is English a first-class locale for the native app? If yes, item 8's lint
  should come before item 3 so new strings are born translated.
- Should `wisp-service` stay as the read path, or is folding reads into the
  loopback host acceptable (debt 7)? This decides whether the home page gets
  live activity status (`activity_source: persisted_only` today).
- Is the macOS 13 deployment target fixed? `@Observable` and
  `NavigationSplitView` improvements need 14; the code uses none of them today,
  so raising the target is optional, not required.
