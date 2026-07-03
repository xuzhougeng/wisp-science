# Project workspace management — design

Date: 2026-07-03

## Problem

Today the app hardcodes a single project (`default`) and a single workspace
directory (`AppState.root`), both fixed at process startup (`src-tauri/src/lib.rs`).
PR #18 made that one directory configurable via env → setting → default, but it
still applies only on next launch and there is exactly one of it.

Users need **project-level isolation**: several projects, each with its own
working directory, chosen at creation time via the OS-native folder picker (so
the path is correct on both macOS and Windows). The app should open onto a
**project management page** — a list of projects to enter — rather than dropping
straight into a chat bound to one global directory.

## Goals

- Landing screen is a Projects page: cards for each project (name, session
  count, last-active) + "New project" + a cross-project "Recent sessions" list.
- New project = enter a name and pick a working directory (native dialog).
- Open a project = switch the active working directory **in-app, no relaunch**;
  skills/memory reload against that directory; the chat view enters that project.
- Delete a project = remove it from the app; **never delete the user's files on
  disk**.
- Existing `default` project and its sessions are preserved (migrated, not lost).

## Non-goals (deferred to issue #10)

Rename, folder grouping, drag-drop reordering, per-project API keys/settings
(keys stay global in the OS keyring), cross-device sync.

## Architecture

### Data model

Add one column to the existing `projects` table:

```sql
ALTER TABLE projects ADD COLUMN workspace_dir TEXT NOT NULL DEFAULT '';
```

The migration runner (`crates/wisp-store/src/lib.rs::migrate`) splits statements
on `;` and is idempotent (`IF NOT EXISTS`), but `ALTER TABLE ADD COLUMN` is not
idempotent, so the new column goes in a separate migration file
(`migrations/0001_project_workspace.sql`) guarded by a `schema_version` check, or
by catching the "duplicate column name" error. Chosen approach: a tiny
`schema_version` setting (`settings` table) gates whether 0001 runs — simplest,
no new migration framework.

The shared SQLite DB (`app_data/wisp.sqlite`) stays global. All projects live in
one DB, isolated by `project_id`; `frames`/`messages`/`artifacts` are already
keyed by `project_id`. API keys stay in the OS keyring, global.

New store methods:

- `list_projects() -> Vec<(id, name, workspace_dir, created_at, updated_at)>`,
  newest-updated first, each annotated with its session count.
- `create_project(id, name, workspace_dir)` (extend the existing signature).
- `get_project(id) -> Option<(name, workspace_dir)>`.
- `delete_project(id)` — `DELETE FROM projects WHERE id=?` (CASCADE clears
  frames/messages/artifacts). Does **not** touch the filesystem.
- `list_recent_sessions(limit) -> Vec<(frame_id, project_id, title, ts)>` —
  cross-project, newest first, for the Recent sessions panel.

`create_project` keeps its `ON CONFLICT DO UPDATE`, extended to update
`workspace_dir`.

### Backend: hot-swappable active project

Move the four startup-fixed fields into an `ActiveProject` held behind a lock:

```rust
#[derive(Clone)]
struct ActiveProject {
    id: String,
    root: PathBuf,
    skills: Arc<SkillIndex>,
    memory: Arc<MemoryManager>,
}

struct AppState {
    app_data: PathBuf,                       // fixed: DB + fallback dirs
    store: Store,                            // fixed: shared DB
    active: tokio::sync::RwLock<ActiveProject>, // swappable
    agent: tokio::sync::Mutex<Option<Agent>>,
    session: tokio::sync::Mutex<SessionState>,
    confirm, bootstrap, cancel,              // unchanged
}
```

`ActiveProject` is cheap to clone (String + PathBuf + two `Arc`s). Add an
accessor:

```rust
impl AppState {
    async fn active(&self) -> ActiveProject { self.active.read().await.clone() }
}
```

Every current read of `state.root` / `state.skills` / `state.memory` /
`state.project_id` (~15 sites: `send_message`, `ensure_frame`, `list_sessions`,
`list_dir`, `read_file`, `upload_file`, `register_artifact`, `build_project_info`,
`get_capabilities`, `list_memory`, etc.) grabs `let ap = state.active().await;`
at the top and uses `ap.root` / `ap.skills` / `ap.memory` / `ap.id`.

The agent is already lazily built and cached in `Mutex<Option<Agent>>`, pulling
`skills`/`memory`/`root`/`project_id` from state at build time (`send_message`
line ~475). So after a swap, setting `*agent = None` makes the next
`send_message` rebuild against the new project automatically — no agent-specific
rebind code.

### Backend: new commands

- `list_projects() -> Vec<ProjectSummary>` — id, name, workspace_dir,
  session_count, updated_at.
- `pick_directory() -> Option<String>` — opens the native folder dialog
  (tauri-plugin-dialog), returns the chosen absolute path.
- `create_project(name, workspace_dir) -> ProjectSummary` — validate the dir is
  creatable/writable (reuse the writability check from `resolve_workspace`
  startup logic), `store.create_project`, return the summary. Does **not** open
  it.
- `open_project(id)` — the switch. Mirrors `new_session`'s cancel dance:
  1. `cancel.store(true)`; acquire `agent` lock (interrupts a running turn,
     per #11/#15); `*agent = None`.
  2. Read the project's `workspace_dir`; ensure it exists/writable (fallback to
     `app_data/workspace` if not, same as startup, and surface a warning).
  3. `skills = SkillIndex::load(&skill_paths(&root))`;
     `memory = MemoryManager::new(&root)`.
  4. `*state.active.write().await = ActiveProject { id, root, skills, memory }`.
  5. Reset `session` to `{ frame_id: None, last_seq: 0 }`; `cancel.store(false)`.
  6. Recompute bootstrap status / project info for the new root.
- `delete_project(id)` — refuse if `id` is the currently-active project (UI
  should force "return to projects" first); else `store.delete_project(id)`.
  Filesystem untouched.

The setup block (`run()`) changes: instead of hardcoding `project_id: "default"`
and one resolved workspace, it resolves the **initial active project**:

- If projects exist, open the project named by the `active_project_id` setting;
  if that setting is missing/stale, fall back to the most-recently-updated
  project. `open_project` writes `active_project_id` so it persists across
  launches.
- Backfill: on first run after this change, the existing `default` project's
  `workspace_dir` is set from the current global `workspace_dir` setting (or the
  resolved startup root), so its sessions stay reachable.
- If no projects exist at all, `active` is seeded with a lightweight
  placeholder pointing at the default workspace, and the UI lands on an empty
  Projects page prompting "New project".

`WISP_WORKSPACE` env override still works: it wins for the initial active
project's root (dev/testing escape hatch).

### Folder picker

Add `tauri-plugin-dialog` to `src-tauri/Cargo.toml`, register the plugin in the
builder, and add its permission to `src-tauri/capabilities/default.json`
(`dialog:allow-open` for directory selection). `pick_directory` calls
`app.dialog().file().blocking_pick_folder()` (or the async variant) and returns
the path string. This is the native picker on each OS — the fix for correct
Mac/Windows paths.

### UI (Leptos)

Add a top-level `screen` signal: `Projects` vs `Chat`. The app boots to
`Projects`.

- **Projects screen** — mirrors the Claude Science reference: header
  "Wisp Science / Beta" + "New project"; a Projects column (cards: name, session
  count, last-active relative time) and a Recent sessions column (cross-project).
  Clicking a card → `open_project(id)` → set `screen = Chat`, refresh
  project_info/sessions.
- **New project** — inline row/modal: name input + "Choose folder" button
  (calls `pick_directory`, shows the chosen path) + Create. Create is disabled
  until both name and a writable folder are set.
- **Delete** — a small control on each card (hover), with a confirm. Copy makes
  clear files on disk are kept.
- **Chat screen** — the existing view, unchanged, except the sidebar's
  `.proj-switch` button now returns to the Projects screen. Capabilities already
  has its own footer button (`open_capabilities`, `main.rs` sidebar footer), so
  repurposing `.proj-switch` loses nothing.

Follow the established visual workflow (per memory note
`ui-claude-science-reference`): iterate layout in `ui/preview.html` +
`ui/styles.css` first, verify in the browser preview, then port structure into
`ui/src/main.rs`. New i18n keys go in `ui/src/i18n.rs` (en + zh).

## Error handling

- Non-writable chosen directory at create time → reject with a clear message;
  don't create the project row.
- Non-writable directory at open time (e.g. offline OneDrive/external drive) →
  fall back to `app_data/workspace`, open anyway, surface a warning (never panic;
  matches PR #18 startup behavior).
- Deleting the active project → rejected; UI returns to Projects first.
- Concurrency: `open_project` takes the `agent` lock and sets `cancel` before
  swapping `active`, so no in-flight turn reads a half-swapped state. `active` is
  an `RwLock`; command handlers take a read guard, clone, and release
  immediately (no guard held across `.await` on other locks).

## Testing

- Store unit tests: `create_project` with workspace_dir round-trips;
  `list_projects` orders by updated_at and counts sessions; `delete_project`
  cascades frames/messages/artifacts and leaves other projects intact;
  `list_recent_sessions` spans projects.
- `resolve_workspace` test stays; add a test that the initial-active-project
  resolution backfills `default`'s workspace_dir from the legacy setting.
- Backend: an `open_project`-style test that swapping `active` and nulling the
  agent causes the next build to use the new root (can be unit-tested at the
  store/state seam without a live Tauri window).
- UI: `cargo check --target wasm32-unknown-unknown` (trunk in scratchpad, per
  memory note) plus a manual pass in preview.html for the Projects screen.

## Migration summary

1. First launch after upgrade: run migration 0001 (add `workspace_dir`), backfill
   `default.workspace_dir` from the global `workspace_dir` setting / resolved
   root.
2. App opens the `default` project (now a real project with a directory); all
   existing sessions remain listed under it.
3. User can create additional projects, each with its own directory, and switch
   between them without relaunching.
