# Project Workspace Management Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the single-workspace app into a multi-project one: a Projects landing page, per-project working directories chosen via the native folder picker, and in-app hot-swap of the active project.

**Architecture:** The four startup-fixed fields (`project_id`, `root`, `skills`, `memory`) move into an `ActiveProject` held behind a `std::sync::RwLock` on `AppState`. Commands read it via a cheap-clone accessor; `open_project` swaps it (after cancelling any running turn and nulling the cached agent, so the next `send_message` rebuilds against the new project). The shared SQLite DB stays global, isolated by `project_id`. A native folder dialog (`tauri-plugin-dialog`) supplies correct absolute paths on macOS/Windows.

**Tech Stack:** Rust, Tauri 2, sqlx/SQLite, Leptos (wasm), tauri-plugin-dialog.

## Global Constraints

- Deleting a project MUST NOT touch the user's files on disk — DB rows only.
- Never panic at startup or on open if a workspace dir is unwritable — fall back to `app_data/workspace` and warn (matches existing PR #18 behavior).
- `WISP_WORKSPACE` env override still wins for the *active project's root at startup* only; it is never persisted into a project row.
- API keys stay global in the OS keyring — no per-project keys.
- `ActiveProject` reads take a read guard, clone, and drop it immediately — never hold the guard across an `.await`.
- UI compile check: `~/.rustup/toolchains/stable-*/bin/cargo check --target wasm32-unknown-unknown` (cargo is not on PATH; a prebuilt `trunk` is in the session scratchpad — see memory note `ui-claude-science-reference`).
- Backend build/test: `cargo build --workspace` and `cargo test --workspace`.
- New user-facing strings get both `en` and `zh` entries in `ui/src/i18n.rs`.

---

### Task 1: Store — schema migration + project methods

**Files:**
- Modify: `crates/wisp-store/migrations/0000_init.sql` (add column to fresh-install CREATE)
- Modify: `crates/wisp-store/src/lib.rs` (idempotent ALTER in `migrate`, extend `create_project`, add `get_project` / `list_projects` / `delete_project` / `list_recent_sessions`, update tests + callers)

**Interfaces:**
- Produces:
  - `Store::create_project(&self, id: &str, name: &str, workspace_dir: &str) -> Result<()>`
  - `Store::get_project(&self, id: &str) -> Result<Option<(String /*name*/, String /*workspace_dir*/)>>`
  - `Store::list_projects(&self) -> Result<Vec<(String /*id*/, String /*name*/, String /*workspace_dir*/, i64 /*created_at*/, i64 /*updated_at*/, i64 /*session_count*/)>>`
  - `Store::delete_project(&self, id: &str) -> Result<()>`
  - `Store::list_recent_sessions(&self, limit: i64) -> Result<Vec<(String /*frame_id*/, String /*project_id*/, String /*title*/, i64 /*ts*/)>>`

- [ ] **Step 1: Add `workspace_dir` to the fresh-install schema**

In `crates/wisp-store/migrations/0000_init.sql`, add the column to the `projects` CREATE:

```sql
CREATE TABLE IF NOT EXISTS projects (
    id            TEXT PRIMARY KEY,
    name          TEXT,
    description   TEXT,
    workspace_dir TEXT NOT NULL DEFAULT '',
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);
```

- [ ] **Step 2: Idempotent ALTER for existing DBs in `migrate`**

In `crates/wisp-store/src/lib.rs`, at the end of `migrate` (before `Ok(())`), add — so DBs created before this change gain the column, while fresh DBs (which already have it) skip it:

```rust
        // Add projects.workspace_dir on DBs that predate it (fresh DBs already
        // have it via 0000_init.sql). pragma_table_info makes this idempotent
        // without a migration-version table.
        let has: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM pragma_table_info('projects') WHERE name='workspace_dir'",
        )
        .fetch_one(pool)
        .await?;
        if has.0 == 0 {
            sqlx::query("ALTER TABLE projects ADD COLUMN workspace_dir TEXT NOT NULL DEFAULT ''")
                .execute(pool)
                .await?;
        }
```

- [ ] **Step 3: Write failing tests for the new methods**

In `crates/wisp-store/src/lib.rs` `mod tests`, add:

```rust
    #[tokio::test]
    async fn project_crud_and_listing() {
        let tmp = std::env::temp_dir().join(format!("wisp_store_proj_{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&tmp).await.unwrap();

        // create + get roundtrips workspace_dir
        store.create_project("a", "Alpha", "/tmp/alpha").await.unwrap();
        store.create_project("b", "Beta", "/tmp/beta").await.unwrap();
        assert_eq!(store.get_project("a").await.unwrap(), Some(("Alpha".into(), "/tmp/alpha".into())));

        // one session under "a" (root frame with a user turn), none under "b"
        store.create_frame("f1", "a", "OPERON", "m").await.unwrap();
        store.append_message("f1", 1, &Message::user("hi")).await.unwrap();

        let projs = store.list_projects().await.unwrap();
        assert_eq!(projs.len(), 2);
        // ordered by updated_at desc; "b" created last so it sorts first
        assert_eq!(projs[0].0, "b");
        let a = projs.iter().find(|p| p.0 == "a").unwrap();
        assert_eq!(a.5, 1, "project a has one session");
        let b = projs.iter().find(|p| p.0 == "b").unwrap();
        assert_eq!(b.5, 0, "project b has no sessions");

        // recent sessions span projects
        store.create_frame("f2", "b", "OPERON", "m").await.unwrap();
        store.append_message("f2", 1, &Message::user("yo")).await.unwrap();
        let recent = store.list_recent_sessions(10).await.unwrap();
        assert_eq!(recent.len(), 2);
        assert!(recent.iter().any(|(_, pid, title, _)| pid == "a" && title == "hi"));

        // delete removes rows for "a" only, leaves "b"
        store.delete_project("a").await.unwrap();
        assert!(store.get_project("a").await.unwrap().is_none());
        assert!(store.load_messages("f1").await.unwrap().is_empty());
        assert!(store.get_project("b").await.unwrap().is_some());
        assert_eq!(store.load_messages("f2").await.unwrap().len(), 1);

        let _ = std::fs::remove_file(&tmp);
    }
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test -p wisp-store project_crud_and_listing -- --nocapture`
Expected: FAIL to compile — `create_project` takes 2 args, `get_project`/`list_projects`/`delete_project`/`list_recent_sessions` don't exist.

- [ ] **Step 5: Extend `create_project` and add the new methods**

In `crates/wisp-store/src/lib.rs`, replace the existing `create_project` with:

```rust
    pub async fn create_project(&self, id: &str, name: &str, workspace_dir: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO projects(id,name,description,workspace_dir,created_at,updated_at) VALUES(?,?,'',?,?,?) \
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, workspace_dir=excluded.workspace_dir, updated_at=excluded.updated_at",
        )
        .bind(id).bind(name).bind(workspace_dir).bind(now).bind(now)
        .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn get_project(&self, id: &str) -> Result<Option<(String, String)>> {
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT COALESCE(name,''), COALESCE(workspace_dir,'') FROM projects WHERE id=?")
                .bind(id).fetch_optional(&self.pool).await?;
        Ok(row)
    }

    /// All projects, newest-updated first, each with its session count
    /// (root frames that have at least one user turn — matches `list_sessions`).
    pub async fn list_projects(&self) -> Result<Vec<(String, String, String, i64, i64, i64)>> {
        let rows = sqlx::query(
            "SELECT p.id AS id, COALESCE(p.name,'') AS name, COALESCE(p.workspace_dir,'') AS ws, \
                    p.created_at AS created_at, p.updated_at AS updated_at, \
                    (SELECT COUNT(*) FROM frames f WHERE f.project_id = p.id AND f.parent_frame_id = f.id \
                       AND EXISTS (SELECT 1 FROM messages m WHERE m.frame_id = f.id AND m.role='user')) AS sessions \
             FROM projects p ORDER BY p.updated_at DESC",
        )
        .fetch_all(&self.pool).await?;
        let mut out = vec![];
        for r in rows {
            out.push((
                r.try_get("id")?, r.try_get("name")?, r.try_get("ws")?,
                r.try_get("created_at")?, r.try_get("updated_at")?, r.try_get("sessions")?,
            ));
        }
        Ok(out)
    }

    /// Delete a project and everything under it. Explicit child deletes (SQLite
    /// FKs are OFF by default, so declared CASCADE would not fire). Filesystem
    /// is untouched — only DB rows.
    /// ponytail: explicit cascade of the 3 known child tables; switch to
    /// `PRAGMA foreign_keys=ON` if more child tables appear.
    pub async fn delete_project(&self, id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM messages WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)").bind(id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM artifacts WHERE project_id=?").bind(id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM frames WHERE project_id=?").bind(id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM projects WHERE id=?").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Newest sessions across ALL projects, for the landing "Recent sessions" list.
    pub async fn list_recent_sessions(&self, limit: i64) -> Result<Vec<(String, String, String, i64)>> {
        let rows = sqlx::query(
            "SELECT f.id AS id, f.project_id AS pid, f.created_at AS created_at, \
                (SELECT content FROM messages m WHERE m.frame_id = f.id AND m.role='user' ORDER BY m.seq ASC LIMIT 1) AS first_user \
             FROM frames f \
             WHERE f.parent_frame_id = f.id \
               AND EXISTS (SELECT 1 FROM messages mm WHERE mm.frame_id = f.id AND mm.role='user') \
             ORDER BY f.created_at DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool).await?;
        let mut out = vec![];
        for row in rows {
            let id: String = row.try_get("id")?;
            let pid: String = row.try_get("pid")?;
            let created: i64 = row.try_get("created_at")?;
            let first_user: Option<String> = row.try_get("first_user")?;
            let title = first_user
                .and_then(|c| serde_json::from_str::<wisp_llm::Content>(&c).ok())
                .map(|c| c.as_text().chars().take(80).collect::<String>())
                .unwrap_or_default();
            out.push((id, pid, title, created));
        }
        Ok(out)
    }
```

- [ ] **Step 6: Fix existing `create_project` callers in store tests**

In the same file's `mod tests`, update every existing `create_project` call to pass a workspace dir. In `roundtrip`, `multi_turn_append`, and `truncate_messages`:

```rust
        store.create_project("p1", "proj", "").await.unwrap();   // roundtrip
        store.create_project("p", "proj", "").await.unwrap();    // multi_turn_append, truncate_messages
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p wisp-store -- --nocapture`
Expected: PASS (all store tests including `project_crud_and_listing`).

- [ ] **Step 8: Commit**

```bash
git add crates/wisp-store
git commit -m "store: per-project workspace_dir + project list/get/delete/recent-sessions"
```

---

### Task 2: Backend — `ActiveProject` hot-swap refactor

This task keeps behavior identical (still opens the `default` project on launch) while making the active project swappable. It compiles and passes existing tests; no new commands yet.

**Files:**
- Modify: `src-tauri/src/lib.rs` (`AppState` struct, add `ActiveProject` + `active()` accessor + `ensure_writable` helper, rewrite ~15 field reads, rewrite `setup` project/workspace resolution)

**Interfaces:**
- Consumes: `Store::create_project(id,name,ws)`, `Store::get_project(id)` (Task 1).
- Produces:
  - `struct ActiveProject { id: String, root: PathBuf, skills: Arc<SkillIndex>, memory: Arc<MemoryManager> }` (derives `Clone`)
  - `AppState.active: std::sync::RwLock<ActiveProject>`
  - `AppState::active(&self) -> ActiveProject` (sync accessor: read-lock, clone, drop)
  - `fn ensure_writable(dir: PathBuf, app_data: &Path) -> PathBuf`

- [ ] **Step 1: Define `ActiveProject`, rewrite `AppState`, add accessor + helper**

In `src-tauri/src/lib.rs`, replace the `AppState` struct (currently fields `root`, `app_data`, `store`, `project_id`, `skills`, `memory`, ...) with:

```rust
#[derive(Clone)]
struct ActiveProject {
    id: String,
    root: PathBuf,
    skills: Arc<SkillIndex>,
    memory: Arc<MemoryManager>,
}

struct AppState {
    app_data: PathBuf,
    store: Store,
    active: std::sync::RwLock<ActiveProject>,
    agent: tokio::sync::Mutex<Option<Agent>>,
    session: tokio::sync::Mutex<SessionState>,
    confirm: Arc<StdMutex<Option<std::sync::mpsc::Sender<bool>>>>,
    bootstrap: StdMutex<BootstrapStatus>,
    cancel: Arc<AtomicBool>,
}

impl AppState {
    /// Snapshot the active project. Cheap: two `Arc` clones + a `String`/`PathBuf`.
    /// Take the guard, clone, drop — never held across `.await`.
    fn active(&self) -> ActiveProject {
        self.active.read().unwrap().clone()
    }
}

/// Ensure `dir` exists and is usable; fall back to `app_data/workspace` if not.
/// Never panics unless even the fallback can't be created.
fn ensure_writable(dir: PathBuf, app_data: &std::path::Path) -> PathBuf {
    if std::fs::create_dir_all(&dir).is_ok() {
        dir
    } else {
        let fallback = app_data.join("workspace");
        tracing::warn!("workspace {:?} not writable; using {:?}", dir, fallback);
        std::fs::create_dir_all(&fallback).expect("create fallback workspace dir");
        fallback
    }
}
```

- [ ] **Step 2: Rewrite every active-field read to go through `active()`**

The refactor rule, applied per function: add `let ap = state.active();` at the top of the body, then replace `state.root`→`ap.root`, `state.skills`→`ap.skills`, `state.memory`→`ap.memory`, `state.project_id`→`ap.id` (as `&ap.id`). For `build_project_info(state: &AppState)` use `let ap = state.active();` likewise.

The sites (from grep — every one must change; the compiler enforces completeness because these fields no longer exist on `AppState`):

- `send_message` (~465): lines 475 (`ap.skills.clone()`, `ap.memory.clone()`, `ap.root.clone()`), 479 (`&ap.id`), 490 (`&ap.skills`), and its `ensure_frame`/`seed_system_prompt` uses at ~601/605.
- `list_sessions` (612): 613 (`&ap.id`).
- `list_skills` (662, **sync fn** — `active()` is sync so no change to signature): 663 (`ap.skills`).
- `load_demo` (672, sync): 673 (`&ap.root`).
- `list_dir` (~810): 813 (`&ap.root`).
- `read_file` (~828): 830 (`&ap.root`).
- `build_project_info` (918): 920, 921, 924 (`&ap.root`), 925 (`&ap.skills`), 927 (`&ap.memory`).
- `get_capabilities` (938): 940 (`&ap.skills`), 943 (`&ap.root`), 944 (`&ap.memory`).
- `list_memory` (950, sync): 951 (`&ap.memory`).
- `register_artifact_at` (1036): 1041 (`&ap.root`), 1043 (`&ap.id`), 1048 (`&ap.id`).
- `upload_file` (1054): 1068, 1070, 1073 (`&ap.root`).

Representative before/after:

```rust
// before (send_message)
let mut agent = Agent::new(cfg.clone(), state.skills.clone(), state.memory.clone(), state.root.clone(), max_context, max_iter);
// after — with `let ap = state.active();` added near the top of send_message
let mut agent = Agent::new(cfg.clone(), ap.skills.clone(), ap.memory.clone(), ap.root.clone(), max_context, max_iter);
```

```rust
// before (upload_file)
let upload_dir = state.root.join("uploads");
let dest = unique_upload_path(&state.root, "uploads", &name);
// after
let ap = state.active();
let upload_dir = ap.root.join("uploads");
let dest = unique_upload_path(&ap.root, "uploads", &name);
```

Note: `build_project_info` currently reads `state.root.file_name()` for the name label — keep it (`ap.root.file_name()`); the on-disk folder name still labels the workspace in the sidebar meta.

- [ ] **Step 3: Rewrite `setup` to resolve + open the initial active project**

In `run()`'s `.setup(...)` closure, replace the block from `let _ = ...create_project("default", "Workspace")` through the `AppState { ... }` construction with:

```rust
            let _ = tauri::async_runtime::block_on(async {
                // Legacy single-workspace installs stored one global `workspace_dir`
                // setting. Backfill the `default` project's dir from it (or the
                // platform default) so its existing sessions stay reachable. Env
                // override is applied to the *root* below, not persisted here.
                let default_workspace = app.path().document_dir()
                    .map(|d| d.join("wisp-science"))
                    .unwrap_or_else(|_| app_data.join("workspace"));
                let legacy_ws = store.get_setting("workspace_dir").await.ok().flatten()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| default_workspace.to_string_lossy().into_owned());
                store.create_project("default", "Workspace", &legacy_ws).await.ok();

                // Pick the initial active project: saved id if it still exists,
                // else "default".
                let active_id = match store.get_setting("active_project_id").await.ok().flatten() {
                    Some(id) if store.get_project(&id).await.ok().flatten().is_some() => id,
                    _ => "default".to_string(),
                };
                let (_, ws) = store.get_project(&active_id).await.ok().flatten()
                    .unwrap_or_else(|| ("Workspace".into(), legacy_ws.clone()));
                (active_id, ws)
            });
```

Wait — capture the returned `(active_id, ws)`:

```rust
            let (active_id, ws) = tauri::async_runtime::block_on(async {
                let default_workspace = app.path().document_dir()
                    .map(|d| d.join("wisp-science"))
                    .unwrap_or_else(|_| app_data.join("workspace"));
                let legacy_ws = store.get_setting("workspace_dir").await.ok().flatten()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| default_workspace.to_string_lossy().into_owned());
                store.create_project("default", "Workspace", &legacy_ws).await.ok();
                let active_id = match store.get_setting("active_project_id").await.ok().flatten() {
                    Some(id) if store.get_project(&id).await.ok().flatten().is_some() => id,
                    _ => "default".to_string(),
                };
                let (_, dir) = store.get_project(&active_id).await.ok().flatten()
                    .unwrap_or_else(|| ("Workspace".into(), legacy_ws.clone()));
                (active_id, dir)
            });

            // Env override wins for the active root only (dev escape hatch; not persisted).
            let default_workspace = app.path().document_dir()
                .map(|d| d.join("wisp-science"))
                .unwrap_or_else(|_| app_data.join("workspace"));
            let root = resolve_workspace(std::env::var("WISP_WORKSPACE").ok(), Some(ws), default_workspace);
            let root = ensure_writable(root, &app_data);

            let skills = Arc::new(SkillIndex::load(&skill_paths(&root)));
            let memory = Arc::new(MemoryManager::new(&root));
            let bootstrap = StdMutex::new(initial_bootstrap(&app_data, &root, skills.all().len()));
            let state = AppState {
                app_data,
                store,
                active: std::sync::RwLock::new(ActiveProject { id: active_id, root, skills, memory }),
                agent: tokio::sync::Mutex::new(None),
                session: tokio::sync::Mutex::new(SessionState { frame_id: None, last_seq: 0 }),
                confirm: Arc::new(StdMutex::new(None)),
                bootstrap,
                cancel: Arc::new(AtomicBool::new(false)),
            };
```

(Remove the old `resolve_workspace(... )` + `create_dir_all`/fallback block and the old `AppState { root, app_data, store, project_id: "default".into(), ... }` — they're replaced above.)

- [ ] **Step 4: Build and run existing tests**

Run: `cargo build --workspace`
Expected: compiles clean (any missed `state.root`/`state.project_id` is a hard error naming the exact line).

Run: `cargo test --workspace`
Expected: PASS (existing tests unchanged in behavior).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "backend: hot-swappable ActiveProject behind RwLock; resolve initial project at startup"
```

---

### Task 3: Backend — project commands + native folder picker

**Files:**
- Modify: `src-tauri/Cargo.toml` (add `tauri-plugin-dialog`)
- Modify: `src-tauri/capabilities/default.json` (add dialog permission)
- Modify: `src-tauri/src/lib.rs` (register plugin, `ProjectSummary`, `build_project_summary`, commands `list_projects` / `pick_directory` / `create_project` / `open_project` / `delete_project`, register them)

**Interfaces:**
- Consumes: `AppState::active()`, `ensure_writable` (Task 2); `Store` project methods (Task 1).
- Produces (Tauri commands, callable from JS `invoke`):
  - `list_projects() -> Vec<ProjectSummary>`
  - `pick_directory() -> Option<String>`
  - `create_project(name: String, workspace_dir: String) -> ProjectSummary`
  - `open_project(id: String) -> ProjectSummary`
  - `delete_project(id: String) -> ()`
  - `ProjectSummary { id, name, workspace_dir, session_count, updated_at }` (Serialize)

- [ ] **Step 1: Add the dialog plugin dependency**

In `src-tauri/Cargo.toml`, under `[dependencies]`:

```toml
tauri-plugin-dialog = "2"
```

- [ ] **Step 2: Grant the dialog permission**

In `src-tauri/capabilities/default.json`, add `"dialog:default"` to the `permissions` array:

```json
  "permissions": ["core:default", "dialog:default"]
```

- [ ] **Step 3: Register the plugin**

In `src-tauri/src/lib.rs`, in `tauri::Builder::default()` chain (before `.setup(`):

```rust
        .plugin(tauri_plugin_dialog::init())
```

- [ ] **Step 4: Add `ProjectSummary` + `build_project_summary`**

Near the other `#[derive(Serialize, Clone)]` structs in `src-tauri/src/lib.rs`:

```rust
#[derive(Serialize, Clone)]
struct ProjectSummary {
    id: String,
    name: String,
    workspace_dir: String,
    session_count: i64,
    updated_at: i64,
}

async fn build_project_summary(state: &AppState, id: &str) -> ProjectSummary {
    // Project counts are tiny; filtering the full list is fine.
    state.store.list_projects().await.ok()
        .and_then(|v| v.into_iter().find(|r| r.0 == id))
        .map(|(id, name, ws, _c, upd, cnt)| ProjectSummary { id, name, workspace_dir: ws, session_count: cnt, updated_at: upd })
        .unwrap_or(ProjectSummary { id: id.into(), name: String::new(), workspace_dir: String::new(), session_count: 0, updated_at: 0 })
}
```

- [ ] **Step 5: Add the commands**

In `src-tauri/src/lib.rs` (near the other `#[tauri::command]` fns):

```rust
#[tauri::command]
async fn list_projects(state: State<'_, AppState>) -> Result<Vec<ProjectSummary>, String> {
    let rows = state.store.list_projects().await.map_err(|e| format!("{e}"))?;
    Ok(rows.into_iter()
        .map(|(id, name, ws, _c, upd, cnt)| ProjectSummary { id, name, workspace_dir: ws, session_count: cnt, updated_at: upd })
        .collect())
}

#[tauri::command]
async fn pick_directory(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |p| { let _ = tx.send(p); });
    let picked = rx.await.map_err(|e| format!("{e}"))?;
    Ok(picked.map(|fp| fp.to_string()))
}

#[tauri::command]
async fn create_project(state: State<'_, AppState>, name: String, workspace_dir: String) -> Result<ProjectSummary, String> {
    if name.trim().is_empty() { return Err("Project name is required".into()); }
    let dir = workspace_dir.trim();
    if dir.is_empty() { return Err("A working directory is required".into()); }
    let path = PathBuf::from(dir);
    std::fs::create_dir_all(&path).map_err(|e| format!("Failed to create working directory: {e}"))?;
    // Writability probe: create + remove a temp marker.
    let marker = path.join(".wisp-write-test");
    std::fs::write(&marker, b"").map_err(|e| format!("Working directory is not writable: {e}"))?;
    let _ = std::fs::remove_file(&marker);

    let id = Uuid::new_v4().to_string();
    state.store.create_project(&id, name.trim(), dir).await.map_err(|e| format!("{e}"))?;
    Ok(build_project_summary(&state, &id).await)
}

#[tauri::command]
async fn open_project(state: State<'_, AppState>, id: String) -> Result<ProjectSummary, String> {
    let (name, ws) = state.store.get_project(&id).await.map_err(|e| format!("{e}"))?
        .ok_or_else(|| "Project not found".to_string())?;
    // Interrupt any running turn and drop the cached agent so the next
    // send_message rebuilds against the new project (mirrors new_session; #11/#15).
    state.cancel.store(true, Ordering::Relaxed);
    { let mut a = state.agent.lock().await; *a = None; }
    let root = ensure_writable(PathBuf::from(&ws), &state.app_data);
    let skills = Arc::new(SkillIndex::load(&skill_paths(&root)));
    let memory = Arc::new(MemoryManager::new(&root));
    { *state.active.write().unwrap() = ActiveProject { id: id.clone(), root: root.clone(), skills, memory }; }
    { *state.session.lock().await = SessionState { frame_id: None, last_seq: 0 }; }
    state.cancel.store(false, Ordering::Relaxed);
    { state.bootstrap.lock().unwrap().workspace = root.to_string_lossy().into_owned(); }
    let _ = state.store.set_setting("active_project_id", &id).await;
    let _ = state.store.create_project(&id, &name, &ws).await; // touch updated_at → sorts to top
    Ok(build_project_summary(&state, &id).await)
}

#[tauri::command]
async fn delete_project(state: State<'_, AppState>, id: String) -> Result<(), String> {
    if state.active().id == id {
        return Err("Return to the projects list before deleting the active project".into());
    }
    state.store.delete_project(&id).await.map_err(|e| format!("{e}"))?;
    Ok(())
}
```

- [ ] **Step 6: Register the commands in `invoke_handler`**

In `tauri::generate_handler![...]`, add: `list_projects, pick_directory, create_project, open_project, delete_project,`.

- [ ] **Step 7: Build**

Run: `cargo build --workspace`
Expected: compiles clean.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/capabilities/default.json src-tauri/src/lib.rs Cargo.lock
git commit -m "backend: project commands (list/create/open/delete) + native folder picker"
```

---

### Task 4: UI — Projects landing screen

Follow the visual workflow from memory note `ui-claude-science-reference`: prototype layout in `ui/preview.html` + `ui/styles.css`, verify in the browser preview, then port structure into `ui/src/main.rs`.

**Files:**
- Modify: `ui/styles.css` (Projects screen styles)
- Modify: `ui/preview.html` (static mock of the Projects screen)
- Modify: `ui/src/i18n.rs` (en + zh keys)
- Modify: `ui/src/main.rs` (`ProjectSummary`/`RecentSession` structs, `show_projects` signal, `ProjectsScreen` component, wire into `App`, repurpose `.proj-switch`)

**Interfaces:**
- Consumes: commands `list_projects`, `pick_directory`, `create_project`, `open_project`, `delete_project` (Task 3).
- Produces: `ProjectsScreen` component; `show_projects: RwSignal<bool>` (true = landing).

- [ ] **Step 1: Add Projects-screen CSS**

In `ui/styles.css`, add (tokens `--clay`, surface/text vars already exist — reuse them; match the reference: two columns, cards with name + meta):

```css
.projects-screen { position: absolute; inset: 0; z-index: 20; background: var(--bg, #f7f6f3);
  overflow-y: auto; padding: 40px 48px; }
.projects-head { display: flex; align-items: flex-start; justify-content: space-between; margin-bottom: 32px; }
.projects-title { font-family: var(--serif); font-size: 34px; line-height: 1.1; }
.projects-title .beta { font-size: 13px; color: var(--muted, #8a8778); display: block; margin-top: 2px; }
.projects-cols { display: grid; grid-template-columns: 1fr 1fr; gap: 40px; max-width: 1200px; }
.projects-col h2 { font-size: 18px; margin-bottom: 14px; display: flex; align-items: center; gap: 8px; }
.proj-card { display: flex; align-items: center; justify-content: space-between; gap: 12px;
  padding: 16px 18px; border-radius: 12px; background: var(--card, #fff);
  border: 1px solid var(--border, #e7e5df); cursor: pointer; margin-bottom: 10px; }
.proj-card:hover { border-color: var(--clay); }
.proj-card .pc-name { font-weight: 600; }
.proj-card .pc-meta { color: var(--muted, #8a8778); font-size: 13px; }
.proj-card .pc-del { opacity: 0; color: var(--muted, #8a8778); background: none; border: 0; cursor: pointer; }
.proj-card:hover .pc-del { opacity: 1; }
.proj-new { border: 1px solid var(--border, #e7e5df); border-radius: 12px; padding: 16px 18px; margin-bottom: 10px; background: var(--card,#fff); }
.proj-new input { width: 100%; padding: 9px 11px; border: 1px solid var(--border,#e7e5df); border-radius: 8px; margin-bottom: 8px; }
.proj-new .pn-dir { display: flex; gap: 8px; align-items: center; }
.proj-new .pn-dir .path { flex: 1; color: var(--muted,#8a8778); font-size: 13px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.btn-primary { background: var(--clay); color: #fff; border: 0; border-radius: 8px; padding: 9px 16px; cursor: pointer; }
.btn-primary:disabled { opacity: .5; cursor: default; }
.btn-ghost { background: none; border: 1px solid var(--border,#e7e5df); border-radius: 8px; padding: 8px 12px; cursor: pointer; }
```

- [ ] **Step 2: Mock the screen in `preview.html` and eyeball it**

In `ui/preview.html`, add a `.projects-screen` block reproducing the layout (title/Beta, "New project" button, a Projects column with 2 sample `.proj-card`s + a `.proj-new` row, a Recent sessions column). Open the browser preview and confirm the two-column layout and card styling match the reference screenshot.

- [ ] **Step 3: Add i18n keys (en + zh)**

In `ui/src/i18n.rs`, add to both locales:

```rust
        ("projects.title", "Projects"),                 // zh: "项目"
        ("projects.new", "New project"),                // zh: "新建项目"
        ("projects.recent", "Recent sessions"),         // zh: "最近会话"
        ("projects.name_ph", "Project name"),           // zh: "项目名称"
        ("projects.choose_dir", "Choose folder"),       // zh: "选择文件夹"
        ("projects.create", "Create"),                  // zh: "创建"
        ("projects.cancel", "Cancel"),                  // zh: "取消"
        ("projects.sessions_n", "{n} sessions"),        // zh: "{n} 个会话"
        ("projects.empty", "No projects yet — create one to start."), // zh: "还没有项目 —— 新建一个开始。"
        ("projects.delete", "Delete"),                  // zh: "删除"
        ("projects.delete_confirm", "Remove this project from Wisp? Your files on disk are kept."), // zh: "从 Wisp 移除该项目？磁盘上的文件会保留。"
        ("projects.back", "Projects"),                  // zh: "项目"
```

- [ ] **Step 4: Add UI data structs + `show_projects` signal**

In `ui/src/main.rs`, near the other JS-facing structs:

```rust
#[derive(Clone, Deserialize)]
struct ProjectSummary {
    id: String,
    name: String,
    #[serde(default)] workspace_dir: String,
    #[serde(default)] session_count: i64,
    #[serde(default)] updated_at: i64,
}

#[derive(Clone, Deserialize)]
struct RecentSession {
    id: String,
    #[allow(dead_code)] project_id: String,
    title: String,
    ts: i64,
}
```

In `App()`, add near the other signals:

```rust
    let show_projects = create_rw_signal(true); // app lands on the Projects screen
```

- [ ] **Step 5: Add the `ProjectsScreen` component**

In `ui/src/main.rs`, add (self-fetches its data; `on_open` runs `open_project` then leaves the screen):

```rust
#[component]
fn ProjectsScreen(locale: RwSignal<String>, on_open: Callback<String>) -> impl IntoView {
    let projects = create_rw_signal(Vec::<ProjectSummary>::new());
    let recent = create_rw_signal(Vec::<RecentSession>::new());
    let creating = create_rw_signal(false);
    let new_name = create_rw_signal(String::new());
    let new_dir = create_rw_signal(String::new());

    let reload = move || {
        spawn_local(async move {
            let v = invoke("list_projects", JsValue::UNDEFINED).await;
            if let Ok(list) = from_value::<Vec<ProjectSummary>>(v) { projects.set(list); }
            let r = invoke("list_recent_sessions", JsValue::UNDEFINED).await;
            if let Ok(list) = from_value::<Vec<RecentSession>>(r) { recent.set(list); }
        });
    };
    reload();

    let choose_dir = move |_| spawn_local(async move {
        let v = invoke("pick_directory", JsValue::UNDEFINED).await;
        if let Ok(Some(p)) = from_value::<Option<String>>(v) { new_dir.set(p); }
    });

    let submit = move |_| {
        let (n, d) = (new_name.get(), new_dir.get());
        if n.trim().is_empty() || d.trim().is_empty() { return; }
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "name": n, "workspaceDir": d })).unwrap();
            let v = invoke("create_project", arg).await;
            if let Ok(p) = from_value::<ProjectSummary>(v) {
                new_name.set(String::new()); new_dir.set(String::new()); creating.set(false);
                on_open.call(p.id);
            }
        });
    };

    let delete = move |id: String| {
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
            let _ = invoke("delete_project", arg).await;
            let v = invoke("list_projects", JsValue::UNDEFINED).await;
            if let Ok(list) = from_value::<Vec<ProjectSummary>>(v) { projects.set(list); }
        });
    };

    view! {
        <div class="projects-screen">
            <div class="projects-head">
                <div class="projects-title">"Wisp Science"<span class="beta">"Beta"</span></div>
                <button class="btn-primary" on:click=move |_| creating.set(true)>
                    {move || t(locale.get(), "projects.new")}
                </button>
            </div>
            <div class="projects-cols">
                <div class="projects-col">
                    <h2>{move || t(locale.get(), "projects.title")}</h2>
                    {move || creating.get().then(|| view! {
                        <div class="proj-new">
                            <input placeholder=move || t(locale.get(), "projects.name_ph")
                                prop:value=move || new_name.get()
                                on:input=move |e| new_name.set(event_target_value(&e)) />
                            <div class="pn-dir">
                                <button class="btn-ghost" on:click=choose_dir>
                                    {move || t(locale.get(), "projects.choose_dir")}
                                </button>
                                <span class="path">{move || new_dir.get()}</span>
                            </div>
                            <div style="display:flex;gap:8px;margin-top:8px">
                                <button class="btn-primary"
                                    disabled=move || new_name.get().trim().is_empty() || new_dir.get().trim().is_empty()
                                    on:click=submit>{move || t(locale.get(), "projects.create")}</button>
                                <button class="btn-ghost" on:click=move |_| creating.set(false)>
                                    {move || t(locale.get(), "projects.cancel")}</button>
                            </div>
                        </div>
                    })}
                    {move || {
                        let loc = locale.get();
                        let list = projects.get();
                        if list.is_empty() && !creating.get() {
                            return view! { <div class="side-hint">{t(loc, "projects.empty")}</div> }.into_view();
                        }
                        list.into_iter().map(|p| {
                            let id_open = p.id.clone();
                            let id_del = p.id.clone();
                            let del = delete.clone();
                            let meta = tf(loc, "projects.sessions_n", &[("n", &p.session_count.to_string())]);
                            view! {
                                <div class="proj-card" on:click=move |_| on_open.call(id_open.clone())>
                                    <div>
                                        <div class="pc-name">{p.name.clone()}</div>
                                        <div class="pc-meta">{meta}</div>
                                    </div>
                                    <button class="pc-del" title=t(loc, "projects.delete")
                                        on:click=move |e| {
                                            e.stop_propagation();
                                            if window().confirm_with_message(t(loc, "projects.delete_confirm")).unwrap_or(false) {
                                                del(id_del.clone());
                                            }
                                        }>"✕"</button>
                                </div>
                            }
                        }).collect_view()
                    }}
                </div>
                <div class="projects-col">
                    <h2>{move || t(locale.get(), "projects.recent")}</h2>
                    {move || recent.get().into_iter().map(|s| view! {
                        <div class="proj-card"><div class="pc-name">{s.title}</div></div>
                    }).collect_view()}
                </div>
            </div>
        </div>
    }
}
```

Note: `list_recent_sessions` is invoked here but was defined on `Store` in Task 1 — expose it as a command. Add to `src-tauri/src/lib.rs` (and to `invoke_handler`):

```rust
#[tauri::command]
async fn list_recent_sessions(state: State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    let rows = state.store.list_recent_sessions(12).await.map_err(|e| format!("{e}"))?;
    Ok(rows.into_iter().map(|(id, pid, title, ts)| serde_json::json!({
        "id": id, "project_id": pid, "title": title, "ts": ts
    })).collect())
}
```

(Fold this small command + its `invoke_handler` registration into Task 3's commit if implementing in order; it's listed here because Task 4 is its only consumer.)

- [ ] **Step 6: Wire `ProjectsScreen` into `App` and repurpose `.proj-switch`**

In `App()`'s `view!`, render the screen as a sibling overlay and hide `.app` under it. Change the outer `<div class="app" ...>` opening tag to add a hidden class, and add the overlay before it:

```rust
    view! {
        {move || show_projects.get().then(|| {
            let open = Callback::new(move |id: String| {
                show_projects.set(false);
                spawn_local(async move {
                    let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                    let _ = invoke("open_project", arg).await;
                    // Reset the chat view for the newly-opened project, then reload
                    // its project info + session list (reuses the existing helpers).
                    items.set(vec![]);
                    active_session.set(None);
                    refresh_sessions(sessions);
                    let v = invoke("get_project_info", JsValue::UNDEFINED).await;
                    if let Ok(p) = serde_wasm_bindgen::from_value::<ProjectInfo>(v) {
                        project_info.set(Some(p));
                    }
                });
            });
            view! { <ProjectsScreen locale=locale on_open=open /> }
        })}
        <div class="app" class:app-hidden=move || show_projects.get() on:contextmenu=on_context_menu>
        // ... unchanged existing body ...
```

Add to `ui/styles.css`: `.app-hidden { display: none; }`.

Repurpose the sidebar project button (`main.rs` ~2082) to return to the Projects screen:

```rust
            <button class="proj-switch" on:click=move |_| show_projects.set(true)>
```

If `refresh_project_info` / `load_sessions` are not already `Callback`s in `App`, use whatever the existing code calls after `new_session` to reload `project_info` and `sessions` (search for `get_project_info` and `list_sessions` invocations in `App`) — call those same spawn_local blocks here.

- [ ] **Step 7: Compile-check the UI**

Run: `~/.rustup/toolchains/stable-*/bin/cargo check --target wasm32-unknown-unknown` (from `ui/`)
Expected: compiles clean.

- [ ] **Step 8: Manual verification**

Build and launch the app (`cargo tauri dev` or the project's run flow). Verify:
1. App opens on the Projects screen showing the migrated `default` project (with its existing session count).
2. "New project" → name + "Choose folder" opens the native OS dialog → pick a dir → Create → app enters the new (empty) project's chat.
3. Sidebar project button returns to Projects; the new project and `default` both appear.
4. Opening `default` shows its old sessions; opening the new one shows none — confirms isolation.
5. Delete a non-active project → it disappears; confirm its directory still exists on disk.

- [ ] **Step 9: Commit**

```bash
git add ui/ src-tauri/src/lib.rs
git commit -m "ui: Projects landing screen with per-project workspace + native folder picker"
```

---

## Self-Review Notes

- **Spec coverage:** schema (T1), hot-swap AppState (T2), commands + folder picker (T3), UI landing + create/open/delete + recent sessions + migration/backfill (T2 setup + T4). All spec sections map to a task.
- **`create_project` name collision:** `Store::create_project` (method) and the `create_project` Tauri command coexist — different scopes; the command calls `state.store.create_project`.
- **FK cascade:** SQLite foreign_keys default OFF, so `delete_project` deletes children explicitly in a transaction rather than relying on declared CASCADE.
- **Sync commands:** `active()` is sync (`std::sync::RwLock`), so `list_skills`/`load_demo`/`list_memory` stay non-async.
- **`list_recent_sessions` command** is consumed only by T4 but defined as a command in T3's file — noted in T4 Step 5 to fold into T3 when building in order.
