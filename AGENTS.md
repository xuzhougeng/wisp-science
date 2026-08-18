# AGENTS.md

## Project Orientation

wisp-science is a Rust/Tauri/Leptos local-first scientific computing agent. The long-term product direction is a research workbench: local, WSL, SSH servers, GPU hosts, schedulers, literature tools, runs, data assets, artifacts, papers, and decisions should be represented as one project-level control plane. The durable product nouns are `Project`, `ExecutionContext`, `DataAsset`, `Run`, `Artifact`, `Paper`, and `Decision`.

Do not implement broad product vision in one change. Prefer small PRs that add one durable abstraction, persistence table, tool, UI surface, or testable behavior at a time.

## Repository Layout

- `crates/wisp-core/`: agent loop, context management, memory, provenance helpers.
- `crates/wisp-tools/`: built-in tools such as read/write/edit/search/grep/shell.
- `crates/wisp-store/`: sqlx SQLite store. Migrations are in `crates/wisp-store/migrations/0000_init.sql`; idempotent migration code lives in `crates/wisp-store/src/lib.rs`.
- `crates/wisp-runtime/`: managed runtime support (currently the persistent Python REPL tool).
- `crates/wisp-skills/`: SKILL.md discovery and use_skill tool.
- `crates/wisp-dto/`: shared serde DTOs for the UI ⇄ Tauri invoke/event contract. Compiles for wasm32 and native; data only, no Leptos/Tauri deps. `ui/src/dto.rs` re-exports it, and `src-tauri/src/dto_contract_tests.rs` deserializes backend payloads into these types to catch serde drift. Add new cross-boundary shapes here, not as hand-mirrored copies.
- `src-tauri/`: desktop shell, Tauri commands, app state, SSH host registry. `src/app_state.rs` owns `AppState`/`SessionRuntime`/`ActiveProject`; `src/agent_turn.rs` owns the send_message turn pipeline, turn queue, and stop_agent; `lib.rs` keeps command registration, setup, and shared helpers.
- `src-tauri/src/model_catalog_shared.rs`: distilled models.dev catalog types and exact-id lookup, compiled into both `build.rs` and the runtime. `build.rs` fetches `https://models.dev/api.json` at build time and falls back to the checked-in `src-tauri/model_catalog.snapshot.json` when offline (`WISP_CATALOG_OFFLINE=1` skips the fetch); run `scripts/refresh_model_catalog.sh` to refresh the snapshot before releases.
- `ui/`: Leptos frontend.
- `ui-tests/`: Playwright tests with mocked Tauri bridge.
- `skills/`: bundled scientific workflows.
- `docs/superpowers/specs/` and `docs/superpowers/plans/`: architecture notes and implementation plans.

## Engineering Rules

- Keep Windows and macOS behavior explicit. Avoid Unix-only assumptions unless gated behind an SSH/WSL context.
- Never require a real SSH host, GPU, SLURM cluster, WSL distro, API key, or network access in automated tests. Use pure parsing tests, fake command runners, temporary directories, and mocked Tauri commands.
- Store secrets in the existing keyring path, not SQLite. SSH private key contents must never be copied into SQLite.
- For long-running compute, do not extend the existing `shell` tool timeout as the main solution. Add a structured run/job abstraction.
- For large scientific data, do not default to local sync. Represent large data as remote references with checksums/metadata where possible.
- Keep schemas backward-compatible and migrations idempotent, following the existing `wisp-store` style.
- Model context/output ceilings come from the baked models.dev catalog via exact model-ID match (gateway `vendor/model` ids match on the tail segment). Never reintroduce prefix or family matching — a family id must not absorb a longer sibling.
- Do not refactor or split modules solely because a file is long. Require a concrete reason tied to the active change, such as mixed responsibilities causing repeated edits, a needed dependency or test boundary, or a measured maintenance problem, and stop once that problem is solved. Large composition/root modules are acceptable; do not pursue arbitrary line-count targets or speculative abstractions.
- Every dismissible overlay, dialog, menu, and popover must participate in a window-level Escape stack ordered from the visually topmost surface down. Root-owned state belongs in the app stack; component-local state may use a scoped window listener that is removed on cleanup. Do not rely on a DOM `keydown` handler receiving a bubbled event or on `autofocus`. A local handler is only appropriate when an inner state must consume Escape before its parent; it must prevent propagation. Tests must press Escape immediately after opening, without first moving focus inside, and verify that one press closes only the topmost layer while its parent remains open.
- UI icons come from one shared set: `compose_icon()` in `ui/src/app_support/messages.rs` (Lucide-style 24×24 stroke SVGs, `stroke="currentColor"`). Never introduce icon fonts, emoji/unicode glyphs, or per-component CSS mask icons (the removed `.gi` system); add a new `compose_icon` kind instead. Give each action a distinct, semantically matching icon — do not reuse the same icon for two different entries in one menu. Size icons via a component-scoped `svg` CSS selector, not by adding wrapper classes to the shared set.
- Add or update tests with every behavior change.
- Update docs when user-visible behavior changes. Update release notes only when explicitly requested or when preparing release-facing changes.
- If `cargo fmt --all -- --check` fails because of formatting drift, run `cargo fmt --all` and keep formatting-only changes in a separate commit.

Grok Build formats each edited `.rs` file via the `PostToolUse` hook in `.grok/hooks/rustfmt-after-edit.json`. That hook does not replace `cargo fmt --all -- --check` on push. Project hooks need `/hooks-trust` (this folder is not trusted by default).

## Verification Commands

Run the narrowest relevant checks first, then the full suite before declaring done:

```bash
cargo fmt --all -- --check
cargo test --workspace
```

For UI or Tauri command changes, also run:

```bash
cd ui && cargo check --target wasm32-unknown-unknown
cd ../ui-tests && npm ci && npx playwright test
```

For MCP-related changes, also run:

```bash
cargo run -p wisp-mcp --example smoke
```

## PR Expectations

Every PR should include:

- A clear statement of the user-facing problem solved.
- A summary of changed files and new abstractions.
- Tests added or updated.
- Manual smoke steps when UI or platform behavior is affected.
- Known limitations and explicit follow-up tasks.

For the research-workbench roadmap, use this ordering:

1. ExecutionContext v0: context registry, SSH/WSL modeling, probe result model, no real long-running jobs yet.
2. Run Manager v1: persisted run/job records, status lifecycle, local/shell/SSH-direct mockable runner, harvest model.
3. Workspace Manifest v1: typed project layout, save/register APIs for scripts/data/results/literature/figures.
4. Research Graph v0: link questions, decisions, data assets, runs, artifacts, and papers.
5. UI integration: contexts panel, runs timeline, artifact/data/literature side panels.
