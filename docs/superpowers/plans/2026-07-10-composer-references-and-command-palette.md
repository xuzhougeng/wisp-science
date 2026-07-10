# Composer References And Command Palette Implementation Plan

**Goal:** Bring the Claude Science-style `@`, `#`, `/`, and Ctrl/Cmd+K interactions to the Wisp chat composer: attach artifacts, attach sessions from any project, explicitly select skills for one turn, and search/open the same catalog from one command palette.

**Reference:** The supplied Claude Science screenshots and the local `/home/xzg/claude-science` binary. The binary is a packaged executable rather than a source checkout, so the screenshots define the user-visible contract; Wisp's existing data model and safety rules define the implementation.

**Architecture:** Reuse the current composer attachment chips, SQLite artifact/session records, `list_skills`, Projects search styling, and `ContextManager::runtime_injections`. Add typed composer references to `send_message`; resolve them in the Tauri host at send time. Do not copy cross-project artifacts, add a search dependency, or add a persistence table in v1.

**Tech Stack:** Rust, Tauri 2, sqlx SQLite, Leptos 0.6 CSR, existing CSS, Playwright with the mocked Tauri bridge.

---

## User-Visible Contract

| Entry | Catalog and scope | Selecting a result | Result grouping |
| --- | --- | --- | --- |
| `@query` | Artifacts from the current conversation, other sessions in the project, then other projects | Removes the trigger token and adds an artifact chip | Current conversation / This project / Other projects |
| `#query` | Saved root sessions from the current and other projects; exclude the active session | Removes the trigger token and adds a session chip | This project / Other projects |
| `/query` | Enabled skills in the active project's effective skill catalog | Removes the trigger token and adds a skill chip for this turn | Featured/bundled first, then other skills |
| Ctrl/Cmd+K | Projects, artifacts, sessions, and a small command list | Enter opens; Shift+Enter attaches artifact/session when a composer is available | Recent/current-project results first, then other projects, then commands |

Shared keyboard behavior:

- Arrow Up/Down changes the active row.
- Enter selects or opens the active row.
- Tab selects an inline `@`, `#`, or `/` result.
- Escape closes only the topmost picker/palette.
- IME composition never opens, navigates, selects, or sends accidentally.
- Duplicate references are ignored; every chip has an explicit remove button.

The composer placeholder becomes: `Ask anything — @ for artifacts, # for sessions, / for skills, Ctrl+K to search…` (and an equivalent Chinese translation). On macOS, visible shortcut copy uses `⌘K`; Windows/Linux use `Ctrl+K`.

## Current Baseline To Reuse

- `ui/src/main.rs::active_mention` and the existing mention menu already implement an end-of-caret `@` picker, keyboard navigation, and attachment chips, but only scan artifacts rendered in the active transcript.
- `search_artifacts` searches the active project; `list_recent_sessions` searches all projects but is fixed to five rows; `list_skills` already returns the effective project catalog.
- `ProjectsScreen` already has the visual structure and open actions for projects, artifacts, sessions, and New session.
- `ContextManager::runtime_injections` provides turn-scoped context which is not written into long-term conversation history.
- `review::serialize_transcript` already produces a UTF-8-safe, recent-tail session transcript capped at 80,000 characters.
- Uploaded files already become artifact records, and the composer already appends file paths to the user request.

## Data Flow

```text
@ / # / slash or Ctrl/Cmd+K
             |
             v
     shared searchable catalog
             |
             v
       typed composer chips
             |
             v
send_message(message, legacy attachments, references[])
             |
             +-- artifact id -> validate DB record -> inject name/path
             +-- session id  -> load saved messages -> inject capped transcript
             +-- skill name   -> validate enabled skill -> inject rendered SKILL.md
             |
             v
        existing agent turn
```

Use one serialized reference shape on the UI/backend boundary:

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
enum ComposerReferenceArg {
    Artifact { id: String },
    Session { id: String },
    Skill { name: String },
}
```

Keep the existing `attachments` argument during the migration for compatibility. New UI sends stable artifact IDs in `references`; paths are resolved by the backend so a stale UI row cannot silently point at the wrong file.

---

## PR 1: Searchable Catalog And Typed Turn Context

### Task 1: Add global artifact and session search rows

**Files:**

- Modify `crates/wisp-store/src/lib.rs`
- Modify `src-tauri/src/lib.rs`
- Modify `ui/src/dto.rs`

- [ ] Add typed store result structs carrying the metadata the picker needs:
  - Artifact: artifact ID, filename, content type, storage path, timestamp, project ID/name/root, root frame ID/title, optional latest-version size, and an origin badge when it can be derived (`upload`, `output`, or generic `artifact`) without a schema change.
  - Session: frame ID/title, project ID/name, activity timestamp, last role/status.
- [ ] Add a global artifact query with an optional project filter. Match filename case-insensitively; an empty query returns newest first.
- [ ] Add a global session query with an optional project filter. Match custom title and first-user title case-insensitively; exclude empty frames.
- [ ] Preserve `search_project_artifacts` and existing command response fields so current callers remain compatible.
- [ ] Expose `search_sessions(query?, limit?, project_id?)` and extend `search_artifacts` with an optional scope/project ID rather than creating one monolithic search endpoint.
- [ ] Make cross-project artifact preview validate the storage path against the artifact owner's project root, not the currently open project root. Preserve the same path-containment checks.
- [ ] Clamp limits in the backend and use parameterized SQL only.

Tests:

- Store tests cover empty-query recency, filename/title filtering, project filtering, project metadata, and empty-frame exclusion.
- Tauri/unit tests cover status mapping and missing optional metadata.

### Task 2: Resolve typed references for one turn

**Files:**

- Modify `src-tauri/src/lib.rs`
- Modify `src-tauri/src/review.rs`
- Modify `crates/wisp-skills/src/tool.rs`
- Modify `crates/wisp-skills/src/lib.rs`
- Modify `ui/src/dto.rs`

- [ ] Add `references: Vec<ComposerReferenceArg>` to `SendMessageArgs` and the Tauri `send_message` command, defaulting to an empty list.
- [ ] Extract the existing `use_skill` rendering into a small public helper and keep `UseSkillTool` using that helper. This avoids duplicating skill body/resource formatting in Tauri.
- [ ] Resolve references after the target session and effective skill index are known, before `agent.run`:
  - Artifact: require a known artifact ID and an existing local file; inject its display name and resolved storage path. Do not copy it into the active project.
  - Session: require a saved root frame other than the target frame; load persisted messages and serialize them as read-only reference material. Running sessions use their latest persisted snapshot only.
  - Skill: require a skill present in the effective enabled catalog; inject the rendered skill guidance/resources as explicit instructions for this turn.
- [ ] Deduplicate while preserving selection order. Cap session references (v1: at most three) and cap their combined injected text to 80,000 characters.
- [ ] Clearly delimit attached transcripts as reference material so instructions quoted inside an old session are not mistaken for the current request.
- [ ] Clear stale runtime injections at the start of a new non-resume send. Keep them across a resumable transient failure, then clear them after successful completion or when a different new turn starts.
- [ ] Return a user-visible error for a deleted/missing artifact, session, or disabled skill instead of silently dropping it.
- [ ] Persist only a concise display suffix in the normal user message (`Uploaded files`, `Attached sessions`, `Selected skills`); the full session/skill content remains runtime-only.

Tests:

- Unit tests cover all three reference types, deduplication, unknown IDs/names, disabled skills, UTF-8-safe caps, transcript delimiting, and cross-project lookup.
- A regression test proves the injected transcript/skill body is absent from persisted messages.
- A resume test proves turn context survives a resumable failure but does not leak into the next fresh turn.

### PR 1 acceptance

- Existing callers can still send with only `message` and `attachments`.
- A backend test can send/resolve an artifact, session, and skill without a real network, SSH host, or model call.
- No SQLite migration or copied cross-project data is introduced.

---

## PR 2: Composer `@`, `#`, And `/` Pickers

### Task 3: Generalize trigger parsing and chip state

**Files:**

- Modify `ui/src/main.rs`
- Modify `ui/src/dto.rs`

- [ ] Replace `active_mention` with a pure `active_composer_trigger` helper that recognizes `@`, `#`, and `/` only at the caret/end of text and only at the start or after whitespace.
- [ ] Keep the current deliberate v1 limit: triggers edited in the middle of the textarea do not open a picker. Document the upgrade path to caret-index scanning.
- [ ] Introduce one typed chip collection for selected artifact/session/skill references while retaining upload progress/error state for file uploads.
- [ ] Selecting a row truncates only the active trigger token, adds a deduplicated chip, closes the picker, and restores composer focus.
- [ ] Sending serializes typed references and clears chips only after the request is accepted for dispatch; editing a prior message strips the concise reference suffix.

Unit tests:

- Trigger detection for all three characters, paths/emails/non-trigger slashes, whitespace, CJK text, and IME-safe key handling helpers.
- Reference dedupe and display-suffix parsing.

### Task 4: Load and render the three catalogs

**Files:**

- Modify `ui/src/main.rs`
- Modify `ui/src/styles/chat.css`
- Modify `ui/src/i18n.rs`

- [ ] Reuse one inline picker shell, flattened row index, and keyboard handler for all modes.
- [ ] `@`: call global `search_artifacts`, current conversation first, current project second, other projects last. Show filename, size when known, and a compact type/source badge.
- [ ] `#`: call `search_sessions`, exclude the active frame, and group current project before other projects. Show title, project, activity time, and status.
- [ ] `/`: cache `list_skills` per active project and filter enabled rows locally by name, description, and tags. Show name and a two-line-clamped description; bundled skills sort first.
- [ ] For async search, apply a response only when its query/mode still matches the active picker, preventing slower old requests from replacing new results.
- [ ] Empty queries show recent/recommended rows; empty result and loading states are localized.
- [ ] Keep the existing native CSS/Leptos implementation and add no picker/fuzzy-search dependency.

### Task 5: Playwright coverage for composer references

**Files:**

- Modify `ui-tests/tests/mock-tauri.ts`
- Modify `ui-tests/tests/ui.spec.ts`

- [ ] Seed artifacts and sessions in the active conversation, another active-project session, and another project.
- [ ] Verify `@`, `#`, and `/` open the correct catalog and groups.
- [ ] Verify mouse and keyboard selection produce removable chips and remove the trigger token.
- [ ] Verify `send_message` receives the exact typed reference payload.
- [ ] Verify Escape, duplicate selection, empty results, and IME composition.

### PR 2 acceptance

- The three triggers work without leaving the keyboard.
- Cross-project artifact/session selection does not switch the active project.
- Selected skills affect only the dispatched turn.
- Existing upload, paste-image, Enter-to-send, and Shift+Enter behavior still passes.

---

## PR 3: Global Ctrl/Cmd+K Palette

### Task 6: Lift the existing Projects search into a shared palette

**Files:**

- Modify `ui/src/main.rs`
- Modify `ui/src/styles/projects.css` (or move the shared rules to `ui/src/styles/overlay.css`)
- Modify `ui/src/i18n.rs`

- [ ] Replace `ProjectsScreen`'s private search overlay with one App-level `CommandPalette` that can open from both the Projects screen and an active project.
- [ ] Reuse the PR 1 search commands and the same catalog row/group types as the composer pickers.
- [ ] Reuse existing open actions:
  - Project -> open project.
  - Session -> open its project, then load the session.
  - Artifact -> preview it without changing projects when the stored path is readable.
  - New session -> existing new-session flow.
- [ ] Add only the commands needed for v1: New session, Project settings (when a project is open), and Manage skills. Do not add a command framework.
- [ ] Enter performs the primary open action. Shift+Enter attaches an artifact/session and focuses the composer when a project is open; otherwise it is disabled and the footer explains why.
- [ ] The existing search toolbar button opens the same palette as Ctrl/Cmd+K.

### Task 7: Global shortcut and overlay priority

**Files:**

- Modify `ui/src/main.rs`
- Modify `ui/src/i18n.rs`

- [ ] Register `(ctrl_key || meta_key) && key == "k"` once at App level, ignore IME composition, call `prevent_default`, and toggle/focus the palette.
- [ ] Put the palette at the top of the Escape stack, ahead of menus, panes, and approval rejection.
- [ ] Arrow navigation wraps or clamps consistently; search changes reset the active row.
- [ ] After close/open/attach, restore focus to the correct surface.

### Task 8: Palette Playwright coverage and docs

**Files:**

- Modify `ui-tests/tests/mock-tauri.ts`
- Modify `ui-tests/tests/ui.spec.ts`
- Modify `README.md`

- [ ] Test Ctrl+K and Meta+K from the Projects screen and active composer.
- [ ] Test keyboard opening of project/session/artifact/command rows.
- [ ] Test Shift+Enter attach, current/other-project grouping, Escape priority, and search-button parity.
- [ ] Document `@`, `#`, `/`, and Ctrl/Cmd+K in the user-facing keyboard/help section.
- [ ] Do not add release notes unless release preparation is requested separately.

### PR 3 acceptance

- One palette implementation serves both app surfaces.
- Search remains local-only and responsive with a large mocked catalog.
- No conflict with composer Enter, existing Escape behavior, or browser/webview defaults.

---

## Full Verification Per PR

Run the narrow tests first, then the repository-required sweep:

```bash
cargo test -p wisp-store
cargo test -p wisp-tauri
cd ui && cargo test
cd .. && cargo fmt --all -- --check
cargo test --workspace
cd ui && cargo check --target wasm32-unknown-unknown
cd ../ui-tests && npm ci && npx playwright test
```

Manual smoke on Windows and macOS:

1. Type each trigger with an empty and non-empty query; select with mouse and keyboard.
2. Attach an artifact and session from another project; confirm the active project does not change and the model receives the referenced context.
3. Select a skill, send once, then send another message without the skill; confirm it is turn-scoped.
4. Open/close the palette with Ctrl+K on Windows and Cmd+K on macOS.
5. Verify Chinese IME Enter confirms a candidate rather than selecting a row or sending.
6. Delete a selected source before send and confirm a clear recoverable error is shown.

## Explicit V1 Limits And Follow-Ups

- Search is case-insensitive lexical matching over names/titles/descriptions, not semantic or fuzzy search. Add FTS only after catalog size/latency data justifies it.
- Session attachment uses the latest persisted, capped transcript snapshot; it does not stream live unsaved deltas.
- Cross-project artifacts remain references to their original local files. Missing files fail clearly; large files are never copied by default.
- Ctrl/Cmd+Enter "open alongside" from the reference UI is not included because Wisp has no side-by-side session surface yet. Add it with that surface, not as a dead shortcut.
- Structured message-to-reference provenance is not persisted in a new table in v1. If research-graph queries need “which turn consumed which artifact/session/skill,” add a backward-compatible `message_context_refs` table as a separate PR.
