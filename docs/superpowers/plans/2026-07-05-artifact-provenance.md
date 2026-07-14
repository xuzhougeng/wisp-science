# Artifact Provenance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Record, for every file a producing tool writes, the code/output/inputs/environment that made it, and surface it in a click-to-open artifact modal with Code / Execution Log / Inputs / Environment panels.

**Architecture:** A snapshot-diff wrapper at wisp-core's single tool-dispatch point (`agent.rs`) computes `files_written`/`files_read` around `python`/`shell` calls and reports a `ProvenanceRecord` through a new `Output::provenance` hook. src-tauri's `TauriOutput` forwards it to an mpsc channel drained by a background task that captures a per-session env snapshot and inserts an `execution_log` row. Provenance is keyed by `(frame_id, workspace-relative path)` — the identifier the UI already uses — and read back by a `get_artifact_provenance` command feeding a new `ArtifactModal`.

**Tech Stack:** Rust (wisp-core, wisp-store, wisp-runtime, Tauri v2), Leptos/WASM UI, sqlx/SQLite, Playwright (mocked bridge) for UI e2e.

## Global Constraints

- Provenance is **best-effort telemetry** — it must NEVER abort or slow a tool call; capture errors are swallowed and logged.
- Keyed by `(frame_id, workspace-relative path)`. **No** `artifacts`-table coupling, no `producing_exec_id` column.
- Producing tools are **`python`** (arg `code`) and **`shell`** (arg `cmd`) only. `language` is tool-derived: `python`→`"python"`, `shell`→`"bash"`.
- `ToolResult` exposes only `success: bool` + `content: String`; store `stdout=content`, `stderr=""`, `exit_status=ok|error`, `wall_s=NULL`.
- No new crate dependencies: hash env snapshots with std `DefaultHasher`, not sha2.
- New-table migrations use `CREATE TABLE IF NOT EXISTS` (idempotent for old + fresh DBs), matching the additive-migration style in `crates/wisp-store/src/lib.rs`.
- Cut (YAGNI): version chains, Messages/Review panels, conda per-cell env detection.

---

### Task 1: wisp-store — provenance schema + query methods

**Files:**
- Modify: `crates/wisp-store/src/lib.rs` (migrations block ~line 60; new `ExecLog` struct + methods; tests in the existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `wisp_store::ExecLog { id, frame_id, cell_index:i64, tool, language, source, stdout, stderr, exit_status, wall_s:Option<f64>, files_written:Vec<String>, files_read:Vec<String>, env_hash:Option<String> }`
- Produces: `Store::next_cell_index(&self, frame_id) -> Result<i64>`, `Store::insert_execution_log(&self, &ExecLog) -> Result<()>`, `Store::record_env_snapshot(&self, hash, env_name:Option<&str>, packages_json) -> Result<()>`, `Store::get_env_snapshot(&self, hash) -> Result<Option<(Option<String>,String)>>`, `Store::find_provenance_by_path(&self, frame_id, path) -> Result<Option<ExecLog>>`, `Store::frame_written_paths(&self, frame_id) -> Result<HashSet<String>>`

- [ ] **Step 1: Add the migrations.** In `Store::open`'s migration section (right after the existing `pragma_table_info` column guards), add:

```rust
        // Provenance: per-cell execution log + env snapshots. CREATE IF NOT EXISTS is
        // idempotent for both fresh and pre-existing DBs (no version table needed).
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS execution_log (\
             id TEXT PRIMARY KEY, frame_id TEXT NOT NULL, cell_index INTEGER NOT NULL, \
             tool TEXT NOT NULL, language TEXT NOT NULL, source TEXT NOT NULL, \
             stdout TEXT, stderr TEXT, exit_status TEXT NOT NULL, wall_s REAL, \
             files_written TEXT NOT NULL, files_read TEXT NOT NULL, env_hash TEXT, \
             created_at INTEGER NOT NULL)",
        ).execute(pool).await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS ix_execution_log_frame ON execution_log(frame_id, cell_index)",
        ).execute(pool).await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS env_snapshots (\
             hash TEXT PRIMARY KEY, env_name TEXT, packages_json TEXT NOT NULL, \
             created_at INTEGER NOT NULL)",
        ).execute(pool).await?;
```

- [ ] **Step 2: Add the `ExecLog` struct + methods.** Near the other artifact methods (after `get_artifact`, ~line 665). `Row`/`try_get` are already imported (used by `get_artifact`).

```rust
#[derive(Debug, Clone, Default)]
pub struct ExecLog {
    pub id: String,
    pub frame_id: String,
    pub cell_index: i64,
    pub tool: String,
    pub language: String,
    pub source: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_status: String,
    pub wall_s: Option<f64>,
    pub files_written: Vec<String>,
    pub files_read: Vec<String>,
    pub env_hash: Option<String>,
}

impl Store {
    /// Next `cell_index` for a frame = count of existing rows.
    pub async fn next_cell_index(&self, frame_id: &str) -> Result<i64> {
        let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM execution_log WHERE frame_id=?")
            .bind(frame_id).fetch_one(&self.pool).await?;
        Ok(n.0)
    }

    pub async fn insert_execution_log(&self, e: &ExecLog) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let fw = serde_json::to_string(&e.files_written).unwrap_or_else(|_| "[]".into());
        let fr = serde_json::to_string(&e.files_read).unwrap_or_else(|_| "[]".into());
        sqlx::query(
            "INSERT INTO execution_log(id,frame_id,cell_index,tool,language,source,stdout,stderr,\
             exit_status,wall_s,files_written,files_read,env_hash,created_at) \
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&e.id).bind(&e.frame_id).bind(e.cell_index).bind(&e.tool).bind(&e.language)
        .bind(&e.source).bind(&e.stdout).bind(&e.stderr).bind(&e.exit_status).bind(e.wall_s)
        .bind(&fw).bind(&fr).bind(&e.env_hash).bind(now)
        .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn record_env_snapshot(
        &self, hash: &str, env_name: Option<&str>, packages_json: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT OR IGNORE INTO env_snapshots(hash,env_name,packages_json,created_at) VALUES(?,?,?,?)",
        )
        .bind(hash).bind(env_name).bind(packages_json).bind(now)
        .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn get_env_snapshot(&self, hash: &str) -> Result<Option<(Option<String>, String)>> {
        let row: Option<(Option<String>, String)> = sqlx::query_as(
            "SELECT env_name, packages_json FROM env_snapshots WHERE hash=?",
        ).bind(hash).fetch_optional(&self.pool).await?;
        Ok(row)
    }

    /// Most-recent execution_log row in `frame_id` whose files_written contains `path`.
    pub async fn find_provenance_by_path(
        &self, frame_id: &str, path: &str,
    ) -> Result<Option<ExecLog>> {
        let rows = sqlx::query(
            "SELECT id,frame_id,cell_index,tool,language,source,stdout,stderr,exit_status,\
             wall_s,files_written,files_read,env_hash FROM execution_log \
             WHERE frame_id=? ORDER BY created_at DESC, cell_index DESC",
        ).bind(frame_id).fetch_all(&self.pool).await?;
        for r in rows {
            let fw: String = r.try_get("files_written")?;
            let written: Vec<String> = serde_json::from_str(&fw).unwrap_or_default();
            if written.iter().any(|p| p == path) {
                let fr: String = r.try_get("files_read")?;
                return Ok(Some(ExecLog {
                    id: r.try_get("id")?,
                    frame_id: r.try_get("frame_id")?,
                    cell_index: r.try_get("cell_index")?,
                    tool: r.try_get("tool")?,
                    language: r.try_get("language")?,
                    source: r.try_get("source")?,
                    stdout: r.try_get("stdout").unwrap_or_default(),
                    stderr: r.try_get("stderr").unwrap_or_default(),
                    exit_status: r.try_get("exit_status")?,
                    wall_s: r.try_get("wall_s").ok(),
                    files_written: written,
                    files_read: serde_json::from_str(&fr).unwrap_or_default(),
                    env_hash: r.try_get("env_hash").ok(),
                }));
            }
        }
        Ok(None)
    }

    /// Union of every path written by any cell in the frame (marks linkable inputs).
    pub async fn frame_written_paths(
        &self, frame_id: &str,
    ) -> Result<std::collections::HashSet<String>> {
        let rows = sqlx::query("SELECT files_written FROM execution_log WHERE frame_id=?")
            .bind(frame_id).fetch_all(&self.pool).await?;
        let mut set = std::collections::HashSet::new();
        for r in rows {
            let fw: String = r.try_get("files_written")?;
            if let Ok(v) = serde_json::from_str::<Vec<String>>(&fw) {
                set.extend(v);
            }
        }
        Ok(set)
    }
}
```

- [ ] **Step 3: Write the test.** Add to `#[cfg(test)] mod tests`:

```rust
    #[tokio::test]
    async fn provenance_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("wisp_prov_{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&tmp).await.unwrap();
        store.create_project("p1", "proj", "").await.unwrap();
        store.create_frame("f1", "p1", "OPERON", "m").await.unwrap();
        store.record_env_snapshot("h1", Some("kernel"), r#"[{"name":"numpy","version":"1.0"}]"#).await.unwrap();
        let e = ExecLog {
            id: "e1".into(), frame_id: "f1".into(), cell_index: 0,
            tool: "python".into(), language: "python".into(),
            source: "savefig('out/fig.png')".into(),
            stdout: "done".into(), stderr: String::new(), exit_status: "ok".into(),
            wall_s: Some(1.5),
            files_written: vec!["out/fig.png".into()],
            files_read: vec!["data.csv".into()],
            env_hash: Some("h1".into()),
        };
        store.insert_execution_log(&e).await.unwrap();
        let got = store.find_provenance_by_path("f1", "out/fig.png").await.unwrap().unwrap();
        assert_eq!(got.source, "savefig('out/fig.png')");
        assert_eq!(got.files_read, vec!["data.csv".to_string()]);
        assert!(store.find_provenance_by_path("f1", "missing.png").await.unwrap().is_none());
        assert_eq!(store.get_env_snapshot("h1").await.unwrap().unwrap().0.as_deref(), Some("kernel"));
        assert!(store.frame_written_paths("f1").await.unwrap().contains("out/fig.png"));
    }
```

- [ ] **Step 4: Run the test.**

Run: `cargo test -p wisp-store provenance_roundtrip`
Expected: PASS. (First confirm it compiles + the store migrations run.)

- [ ] **Step 5: Commit.**

```bash
git add crates/wisp-store/src/lib.rs
git commit -m "feat(store): execution_log + env_snapshots tables and provenance queries"
```

---

### Task 2: wisp-core — snapshot/diff module + `Output::provenance` + agent wiring

**Files:**
- Create: `crates/wisp-core/src/provenance.rs`
- Modify: `crates/wisp-core/src/lib.rs` (declare `pub mod provenance;` + `pub use provenance::ProvenanceRecord;`)
- Modify: `crates/wisp-core/src/output.rs` (add `fn provenance` to the `Output` trait)
- Modify: `crates/wisp-core/src/agent.rs` (wrap the `tools.run` dispatch)

**Interfaces:**
- Consumes: nothing from Task 1 (pure wisp-core).
- Produces: `wisp_core::ProvenanceRecord { tool, language, source, output, success:bool, files_written:Vec<String>, files_read:Vec<String> }`; `Output::provenance(&self, &ProvenanceRecord)`; `provenance::{is_producing, language_of, source_of, snapshot, diff}`.

- [ ] **Step 1: Write the failing test** (create `crates/wisp-core/src/provenance.rs` with the module + its test):

```rust
//! Best-effort artifact provenance: snapshot the workspace around a producing
//! tool call and diff to learn which files it wrote and read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Reported to `Output::provenance` after a producing tool writes ≥1 file.
#[derive(Debug, Clone, Default)]
pub struct ProvenanceRecord {
    pub tool: String,
    pub language: String,
    pub source: String,
    pub output: String,
    pub success: bool,
    pub files_written: Vec<String>,
    pub files_read: Vec<String>,
}

const SKIP_DIRS: &[&str] = &[".git", ".venv", "node_modules", ".wisp", "uploads", "__pycache__"];
// ponytail: recursive mtime scan, capped + heavy dirs skipped. Swap for an fs-notify
// watcher only if this shows up in a profile.
const MAX_FILES: usize = 20_000;

pub fn is_producing(tool: &str) -> bool {
    matches!(tool, "python" | "shell")
}

pub fn language_of(tool: &str) -> String {
    match tool {
        "python" => "python",
        "shell" => "bash",
        _ => "text",
    }
    .to_string()
}

pub fn source_of(tool: &str, args: &serde_json::Value) -> String {
    let key = if tool == "python" { "code" } else { "cmd" };
    args.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}

/// Recursive path→mtime map of the workspace, skipping heavy dirs, capped.
pub fn snapshot(root: &Path) -> BTreeMap<PathBuf, SystemTime> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if out.len() >= MAX_FILES {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            let p = entry.path();
            if ft.is_dir() {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !SKIP_DIRS.contains(&name) {
                    stack.push(p);
                }
            } else if ft.is_file() {
                let mtime = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                out.insert(p, mtime);
            }
        }
    }
    out
}

/// Diff two snapshots → (files_written, files_read), both workspace-relative.
/// written = new or mtime-advanced files; read = pre-existing files (not also written)
/// whose relative path appears literally in `source`.
pub fn diff(
    before: &BTreeMap<PathBuf, SystemTime>,
    after: &BTreeMap<PathBuf, SystemTime>,
    root: &Path,
    source: &str,
) -> (Vec<String>, Vec<String>) {
    let rel = |p: &Path| -> String {
        p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/")
    };
    let mut written = Vec::new();
    for (p, mt) in after {
        match before.get(p) {
            None => written.push(rel(p)),
            Some(old) if mt > old => written.push(rel(p)),
            _ => {}
        }
    }
    written.sort();
    let wset: std::collections::HashSet<&String> = written.iter().collect();
    let mut read = Vec::new();
    for p in before.keys() {
        let r = rel(p);
        if !r.is_empty() && !wset.contains(&r) && source.contains(&r) {
            read.push(r);
        }
    }
    read.sort();
    (written, read)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_written_and_read_and_skips_git() {
        let tmp = std::env::temp_dir().join("wisp_prov_snap_test");
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(tmp.join(".git")).unwrap();
        std::fs::write(tmp.join("data.csv"), b"x").unwrap();
        std::fs::write(tmp.join(".git/HEAD"), b"x").unwrap();
        let before = snapshot(&tmp);
        assert!(!before.keys().any(|p| p.ends_with("HEAD")), ".git must be skipped");
        std::fs::write(tmp.join("out.png"), b"y").unwrap();
        let after = snapshot(&tmp);
        let (w, r) = diff(&before, &after, &tmp, "df=read_csv('data.csv'); savefig('out.png')");
        assert!(w.contains(&"out.png".to_string()));
        assert!(r.contains(&"data.csv".to_string()));
        std::fs::remove_dir_all(&tmp).ok();
    }
}
```

- [ ] **Step 2: Declare the module + export.** In `crates/wisp-core/src/lib.rs`, add alongside the other `mod` declarations:

```rust
pub mod provenance;
pub use provenance::ProvenanceRecord;
```

- [ ] **Step 3: Run the test to verify it passes.**

Run: `cargo test -p wisp-core detects_written_and_read_and_skips_git`
Expected: PASS.

- [ ] **Step 4: Add the `Output::provenance` hook.** In `crates/wisp-core/src/output.rs`, add to the `Output` trait (after `on_message`, before the closing `}`):

```rust
    /// Fired once per producing tool call that wrote ≥1 file, with the code,
    /// result text, and diffed inputs/outputs. Default: no-op (CLI ignores it).
    fn provenance(&self, _rec: &crate::provenance::ProvenanceRecord) {}
```

- [ ] **Step 5: Wire the dispatch wrapper.** In `crates/wisp-core/src/agent.rs`, add `use crate::provenance;` to the imports, then replace the single line `let result = tools.run(&name, &args, &env).await;` with:

```rust
            let producing = provenance::is_producing(&name);
            let before = if producing {
                provenance::snapshot(env.project_root())
            } else {
                Default::default()
            };
            let result = tools.run(&name, &args, &env).await;
            if producing {
                let after = provenance::snapshot(env.project_root());
                let source = provenance::source_of(&name, &args);
                let (written, read) = provenance::diff(&before, &after, env.project_root(), &source);
                if !written.is_empty() {
                    output.provenance(&provenance::ProvenanceRecord {
                        tool: name.clone(),
                        language: provenance::language_of(&name),
                        source,
                        output: result.content.clone(),
                        success: result.success,
                        files_written: written,
                        files_read: read,
                    });
                }
            }
```

- [ ] **Step 6: Verify it compiles.**

Run: `cargo check -p wisp-core`
Expected: no errors (CLI's `Output` impl inherits the default `provenance` no-op).

- [ ] **Step 7: Commit.**

```bash
git add crates/wisp-core/src/provenance.rs crates/wisp-core/src/lib.rs crates/wisp-core/src/output.rs crates/wisp-core/src/agent.rs
git commit -m "feat(core): snapshot-diff provenance capture + Output::provenance hook"
```

---

### Task 3: src-tauri — persist provenance (drain task, env snapshot, TauriOutput hook)

**Files:**
- Modify: `src-tauri/src/lib.rs` (`TauriOutput` struct + `impl Output`; `run_turn`'s channel/drain setup near line 1556-1594; new `parse_pip_list`/`capture_env` helpers; a `#[cfg(test)]` test)

**Interfaces:**
- Consumes: `wisp_store::ExecLog`, `Store::{next_cell_index, insert_execution_log, record_env_snapshot}` (Task 1); `wisp_core::ProvenanceRecord`, `Output::provenance` (Task 2); `wisp_runtime::PythonEnv::{find_uv, python}`.
- Produces: persisted `execution_log` + `env_snapshots` rows during a turn. `parse_pip_list(&str) -> Vec<PipPkg>`.

- [ ] **Step 1: Write the failing test** for the pure env-parse helper. Add near the bottom of `src-tauri/src/lib.rs`:

```rust
#[cfg(test)]
mod provenance_tests {
    use super::*;
    #[test]
    fn parse_pip_list_reads_name_version() {
        let json = r#"[{"name":"numpy","version":"1.26.0"},{"name":"pandas","version":"2.2.0"}]"#;
        let pkgs = parse_pip_list(json);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "numpy");
        assert_eq!(pkgs[1].version, "2.2.0");
        assert!(parse_pip_list("not json").is_empty());
    }
}
```

- [ ] **Step 2: Add the env helpers.** Add near the other free functions in `src-tauri/src/lib.rs` (e.g. above `register_artifact_at`):

```rust
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct PipPkg {
    name: String,
    #[serde(default)]
    version: String,
}

/// Parse `uv pip list --format=json` / `pip list --format=json` output.
fn parse_pip_list(json: &str) -> Vec<PipPkg> {
    serde_json::from_str::<Vec<PipPkg>>(json).unwrap_or_default()
}

/// Capture the kernel venv's package list once; store it hashed; return the hash.
/// Non-fatal: any failure returns `None` and the Environment panel shows "unavailable".
async fn capture_env(store: &wisp_store::Store, app_data: &std::path::Path) -> Option<String> {
    let venv = app_data.join("python").join(".venv");
    let python = wisp_runtime::PythonEnv { venv }.python();
    let uv = wisp_runtime::PythonEnv::find_uv()?;
    let out = tokio::process::Command::new(&uv)
        .args(["pip", "list", "--format=json", "--python"])
        .arg(&python)
        .output()
        .await
        .ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    let json = String::from_utf8_lossy(&out.stdout).into_owned();
    let packages = parse_pip_list(&json);
    if packages.is_empty() {
        return None;
    }
    let packages_json = serde_json::to_string(&packages).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&packages_json, &mut h);
    let hash = format!("{:016x}", std::hash::Hasher::finish(&h));
    store.record_env_snapshot(&hash, Some("kernel"), &packages_json).await.ok()?;
    Some(hash)
}
```

- [ ] **Step 3: Add the `prov` field to `TauriOutput` + implement `provenance`.** In the `TauriOutput` struct (near line 586+ / the struct with `persist`), add a field:

```rust
    prov: Option<tokio::sync::mpsc::UnboundedSender<wisp_core::ProvenanceRecord>>,
```

In `impl Output for TauriOutput`, add the method:

```rust
    fn provenance(&self, rec: &wisp_core::ProvenanceRecord) {
        if let Some(tx) = &self.prov {
            let _ = tx.send(rec.clone());
        }
    }
```

- [ ] **Step 4: Spawn the drain task + wire the channel.** In `run_turn`, immediately after the existing `(persist_handle, persist_tx)` block (line ~1575) and before `let output = TauriOutput {`, add:

```rust
    let (prov_handle, prov_tx) = {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<wisp_core::ProvenanceRecord>();
        let store = state.store.clone();
        let app_data = state.app_data.clone();
        let fid = frame_id.clone();
        let handle = tokio::spawn(async move {
            let mut env_hash: Option<String> = None;
            while let Some(rec) = rx.recv().await {
                if env_hash.is_none() {
                    env_hash = capture_env(&store, &app_data).await;
                }
                let cell_index = store.next_cell_index(&fid).await.unwrap_or(0);
                let e = wisp_store::ExecLog {
                    id: Uuid::new_v4().to_string(),
                    frame_id: fid.clone(),
                    cell_index,
                    tool: rec.tool,
                    language: rec.language,
                    source: rec.source,
                    stdout: rec.output,
                    stderr: String::new(),
                    exit_status: if rec.success { "ok".into() } else { "error".into() },
                    wall_s: None,
                    files_written: rec.files_written,
                    files_read: rec.files_read,
                    env_hash: env_hash.clone(),
                };
                if let Err(e) = store.insert_execution_log(&e).await {
                    tracing::warn!("provenance persist failed: {e}");
                }
            }
        });
        (handle, tx)
    };
```

Then add `prov: Some(prov_tx),` to the `TauriOutput { ... }` literal.

- [ ] **Step 5: Join the drain task on turn end.** After the existing `drop(output);` + persist-handle await block (line ~1593), add:

```rust
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), prov_handle).await;
```

(`drop(output)` already closes `prov_tx` since `output` owns it, ending the drain loop.)

- [ ] **Step 6: Run the test + compile.**

Run: `cargo test -p wisp-tauri parse_pip_list_reads_name_version && cargo check -p wisp-tauri`
Expected: test PASS, check clean.

- [ ] **Step 7: Commit.**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(tauri): persist provenance records + per-session env snapshot"
```

---

### Task 4: src-tauri — `get_artifact_provenance` command

**Files:**
- Modify: `src-tauri/src/lib.rs` (new command + serde types + `to_workspace_rel` helper; register in `invoke_handler`)

**Interfaces:**
- Consumes: `Store::{find_provenance_by_path, frame_written_paths, get_env_snapshot}` (Task 1).
- Produces: command `get_artifact_provenance(session_id: Option<String>, path: String) -> Result<Option<ArtifactProvenance>, String>` returning `ArtifactProvenance { code, language, output, exit_status, inputs:[{path, produced_here}], env:Option<{name, packages:[{name,version}]}> }`.

- [ ] **Step 1: Add the serde types + helper.** In `src-tauri/src/lib.rs`:

```rust
#[derive(serde::Serialize)]
struct ProvInput {
    path: String,
    produced_here: bool,
}
#[derive(serde::Serialize)]
struct ProvEnv {
    name: Option<String>,
    packages: Vec<PipPkg>,
}
#[derive(serde::Serialize)]
struct ArtifactProvenance {
    code: String,
    language: String,
    output: String,
    exit_status: String,
    inputs: Vec<ProvInput>,
    env: Option<ProvEnv>,
}

/// Normalize a UI path (absolute or relative) to the workspace-relative form used
/// in `execution_log.files_written`.
fn to_workspace_rel(root: &std::path::Path, path: &str) -> String {
    let p = std::path::Path::new(path);
    p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/")
}
```

- [ ] **Step 2: Add the command.**

```rust
/// Provenance for a produced artifact, addressed by workspace path. `None` when the
/// path has no recorded producing cell (uploads, pre-feature figures) → empty modal.
#[tauri::command]
async fn get_artifact_provenance(
    state: State<'_, AppState>,
    session_id: Option<String>,
    path: String,
) -> Result<Option<ArtifactProvenance>, String> {
    let frame_id = match session_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => Some(id.to_string()),
        None => state.active_frame.read().unwrap().clone(),
    };
    let Some(fid) = frame_id else { return Ok(None) };
    let ap = state.active();
    let rel = to_workspace_rel(&ap.root, &path);
    let Some(e) = state
        .store
        .find_provenance_by_path(&fid, &rel)
        .await
        .map_err(|e| format!("{e}"))?
    else {
        return Ok(None);
    };
    let written = state.store.frame_written_paths(&fid).await.unwrap_or_default();
    let inputs = e
        .files_read
        .iter()
        .map(|p| ProvInput { path: p.clone(), produced_here: written.contains(p) })
        .collect();
    let env = match e.env_hash.as_deref() {
        Some(h) => state
            .store
            .get_env_snapshot(h)
            .await
            .ok()
            .flatten()
            .map(|(name, pj)| ProvEnv { name, packages: parse_pip_list(&pj) }),
        None => None,
    };
    Ok(Some(ArtifactProvenance {
        code: e.source,
        language: e.language,
        output: e.stdout,
        exit_status: e.exit_status,
        inputs,
        env,
    }))
}
```

- [ ] **Step 3: Register the command.** In the `tauri::generate_handler![...]` list (near line 3489 where `list_artifacts`, `register_artifact` are), add `get_artifact_provenance,`.

- [ ] **Step 4: Compile.**

Run: `cargo check -p wisp-tauri`
Expected: clean.

- [ ] **Step 5: Commit.**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(tauri): get_artifact_provenance command (lookup by workspace path)"
```

---

### Task 5: UI — ArtifactModal (click image → tabbed provenance) + i18n + e2e

**Files:**
- Modify: `ui/src/dto.rs` (add `ArtifactProvenance` + nested types)
- Modify: `ui/src/main.rs` (`ArtifactModal` component; `modal_artifact` signal; make the artifact/file preview clickable; render the modal)
- Modify: `ui/src/i18n.rs` (En + Zh keys)
- Modify: `ui/src/styles/modals.css` (or `right-pane.css`) — modal-specific classes
- Modify: `ui-tests/tests/mock-tauri.ts` (mock `get_artifact_provenance`)
- Modify: `ui-tests/tests/ui.spec.ts` (new e2e)

**Interfaces:**
- Consumes: command `get_artifact_provenance` (Task 4).
- Produces: `ArtifactModal` opened via `modal_artifact: RwSignal<Option<(String path, String name, String kind)>>`.

- [ ] **Step 1: Add DTO types.** In `ui/src/dto.rs`:

```rust
#[derive(Clone, serde::Deserialize, Default)]
pub(crate) struct ArtifactProvenance {
    pub(crate) code: String,
    pub(crate) language: String,
    pub(crate) output: String,
    pub(crate) exit_status: String,
    #[serde(default)]
    pub(crate) inputs: Vec<ProvInput>,
    pub(crate) env: Option<ProvEnv>,
}
#[derive(Clone, serde::Deserialize)]
pub(crate) struct ProvInput {
    pub(crate) path: String,
    pub(crate) produced_here: bool,
}
#[derive(Clone, serde::Deserialize)]
pub(crate) struct ProvEnv {
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) packages: Vec<ProvPkg>,
}
#[derive(Clone, serde::Deserialize)]
pub(crate) struct ProvPkg {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) version: String,
}
```

- [ ] **Step 2: Add i18n keys.** In `ui/src/i18n.rs`, add to the En block (near `right.*`) and the Zh block:

```rust
        // En
        (Locale::En, "artifact.expand") => Some("View full size"),
        (Locale::En, "artifact.tab.code") => Some("Code"),
        (Locale::En, "artifact.tab.log") => Some("Execution Log"),
        (Locale::En, "artifact.tab.inputs") => Some("Inputs"),
        (Locale::En, "artifact.tab.env") => Some("Environment"),
        (Locale::En, "artifact.none") => Some("No provenance recorded for this file."),
        (Locale::En, "artifact.env.none") => Some("Environment unavailable."),
        (Locale::En, "artifact.download") => Some("Download"),
```

```rust
        // Zh
        (Locale::Zh, "artifact.expand") => Some("查看大图"),
        (Locale::Zh, "artifact.tab.code") => Some("代码"),
        (Locale::Zh, "artifact.tab.log") => Some("执行记录"),
        (Locale::Zh, "artifact.tab.inputs") => Some("输入"),
        (Locale::Zh, "artifact.tab.env") => Some("环境"),
        (Locale::Zh, "artifact.none") => Some("该文件没有记录来源信息。"),
        (Locale::Zh, "artifact.env.none") => Some("环境信息不可用。"),
        (Locale::Zh, "artifact.download") => Some("下载"),
```

- [ ] **Step 3: Add the `ArtifactModal` component.** In `ui/src/main.rs` (near the other `#[component]`s / preview code):

```rust
#[component]
fn ArtifactModal(
    path: String,
    name: String,
    kind: String,
    session: Option<String>,
    on_close: Callback<()>,
    on_open_path: Callback<(String, String)>, // open an input file (path, kind)
) -> impl IntoView {
    let locale = use_locale();
    let prov = create_rw_signal(None::<ArtifactProvenance>);
    let loaded = create_rw_signal(false);
    let tab = create_rw_signal("code");
    let dom_id = unique_dom_id("amodal");
    // Fetch provenance once.
    {
        let path = path.clone();
        let session = session.clone();
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "sessionId": session, "path": path })).unwrap();
            let v = invoke("get_artifact_provenance", arg).await;
            prov.set(serde_wasm_bindgen::from_value::<Option<ArtifactProvenance>>(v).ok().flatten());
            loaded.set(true);
        });
    }
    let path_head = path.clone();
    view! {
        <div class="overlay" on:click=move |_| on_close.call(())>
            <div class="modal artifact-modal" on:click=|ev| ev.stop_propagation()>
                <div class="am-head">
                    <span class="am-name">{name.clone()}</span>
                    <div class="spacer"></div>
                    <button class="icon-btn" title=move || t(locale.get(), "artifact.download")
                        on:click={let p = path.clone(); move |_| { let p = p.clone(); spawn_local(async move {
                            let arg = to_value(&serde_json::json!({ "path": p })).unwrap();
                            let _ = invoke("reveal_in_os", arg).await; }); }}>"⭳"</button>
                    <button class="icon-btn" title=move || t(locale.get(), "right.close")
                        on:click=move |_| on_close.call(())>"×"</button>
                </div>
                <div class="am-figure">
                    {if kind == "image" || kind == "pdf" {
                        view! { <FilePreview dom_id=dom_id path=path_head.clone() kind=kind.clone() /> }.into_view()
                    } else {
                        view! { <p class="rp-path hint">{path_head.clone()}</p> }.into_view()
                    }}
                </div>
                <div class="am-tabs">
                    {["code","log","inputs","env"].iter().map(|k| {
                        let k = *k;
                        let label_key = format!("artifact.tab.{k}");
                        view! {
                            <button class="am-tab" class:active=move || tab.get()==k
                                on:click=move |_| tab.set(k)>
                                {move || t(locale.get(), &label_key)}</button>
                        }
                    }).collect_view()}
                </div>
                <div class="am-panel">
                    {move || {
                        let loc = locale.get();
                        if !loaded.get() { return view! { <div class="rp-heavy">{t(loc,"loading")}</div> }.into_view(); }
                        let Some(p) = prov.get() else {
                            return view! { <div class="am-empty">{t(loc,"artifact.none")}</div> }.into_view();
                        };
                        match tab.get() {
                            "code" => view! { <RpCodeView lang=p.language.clone() body=p.code.clone() /> }.into_view(),
                            "log" => view! { <pre class="am-log">{p.output.clone()}</pre> }.into_view(),
                            "inputs" => view! {
                                <div class="am-inputs">
                                    {p.inputs.iter().map(|i| {
                                        let ip = i.path.clone();
                                        let linkable = i.produced_here;
                                        let open = on_open_path;
                                        view! {
                                            <button class="am-input" class:linkable=linkable
                                                on:click=move |_| if linkable { open.call((ip.clone(), file_kind_for(&ip))) }>
                                                {i.path.clone()}</button>
                                        }
                                    }).collect_view()}
                                </div>
                            }.into_view(),
                            _ => match p.env.clone() {
                                None => view! { <div class="am-empty">{t(loc,"artifact.env.none")}</div> }.into_view(),
                                Some(env) => view! {
                                    <table class="am-env">
                                        {env.packages.iter().map(|pk| view! {
                                            <tr><td>{pk.name.clone()}</td><td>{pk.version.clone()}</td></tr>
                                        }).collect_view()}
                                    </table>
                                }.into_view(),
                            },
                        }
                    }}
                </div>
            </div>
        </div>
    }
}
```

Notes for the implementer:
- `file_kind_for(path: &str) -> String` — reuse the existing kind-from-extension helper used by `collect_markdown_artifacts` (search for where `PreviewData::File { kind }` gets its `kind`; extract/reuse it). If none is factored out, add `fn file_kind_for(path:&str)->String` mapping `.png/.jpg/.svg→"image"`, `.pdf→"pdf"`, `.csv→"csv"`, else `"text"`.
- `reveal_in_os` — reuse whatever existing command opens/reveals a file (search the invoke_handler list); if only a "reveal" exists, that is acceptable for v1 download. If none exists, drop the download button for v1 (note it, add later).

- [ ] **Step 4: Add the `modal_artifact` signal + render the modal + open triggers.** In the main App component, near the other right-pane signals (`sel_artifact`, `open_file`):

```rust
    let modal_artifact = create_rw_signal(None::<(String, String, String)>); // (path, name, kind)
```

Wrap the artifact preview in `.rp-view` (line ~3788, the block that renders `{artifact_preview(&cur, dom_id, loc)}`) so clicking an image opens the modal — add an expand button to `.rp-view-head` for file-backed image/pdf artifacts:

```rust
                                                    <div class="rp-view-head">
                                                        <span class=format!("rp-badge {}", cur.kind)>{cur.kind.to_string()}</span>
                                                        <span class="rp-view-name">{cur.name.clone()}</span>
                                                        {matches!(&cur.data, PreviewData::File { kind, .. } if kind=="image"||kind=="pdf").then(|| {
                                                            let (name, data) = (cur.name.clone(), cur.data.clone());
                                                            view! {
                                                                <div class="spacer"></div>
                                                                <button class="icon-btn" title=move || t(locale.get(), "artifact.expand")
                                                                    on:click=move |_| if let PreviewData::File { path, kind } = &data {
                                                                        modal_artifact.set(Some((path.clone(), name.clone(), kind.clone())));
                                                                    }>"⤢"</button>
                                                            }
                                                        })}
                                                    </div>
```

Render the modal once at the app root, next to the other modals (e.g. after the `show_proj_settings` modal block):

```rust
        {move || modal_artifact.get().map(|(path, name, kind)| {
            let session = active_session.get();
            view! {
                <ArtifactModal path=path name=name kind=kind session=session
                    on_close=Callback::new(move |_| modal_artifact.set(None))
                    on_open_path=Callback::new(move |(p, k): (String, String)| {
                        open_file.set(Some((p, k)));
                        right_tab.set(RightTab::File);
                        modal_artifact.set(None);
                    }) />
            }
        })}
```

(`active_session`, `open_file`, `right_tab` are existing signals in that scope.)

- [ ] **Step 5: Add modal CSS.** In `ui/src/styles/modals.css`:

```css
.artifact-modal { max-width: min(880px, 94vw); width: 100%; gap: 10px; }
.artifact-modal .am-head { display: flex; align-items: center; gap: 8px; }
.artifact-modal .am-name { font-weight: 600; font-size: 14px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.artifact-modal .am-figure { max-height: 52vh; overflow: auto; display: flex; justify-content: center; background: var(--bg-sunken); border-radius: var(--radius-sm); padding: 8px; }
.artifact-modal .am-figure img, .artifact-modal .am-figure canvas { max-width: 100%; height: auto; }
.artifact-modal .am-tabs { display: flex; gap: 6px; border-bottom: 1px solid var(--border-strong); }
.artifact-modal .am-tab { padding: 6px 10px; border: none; background: transparent; color: var(--text-faint); font: inherit; font-size: 12px; cursor: pointer; border-radius: 8px 8px 0 0; }
.artifact-modal .am-tab.active { background: var(--bg-sunken); color: var(--text); }
.artifact-modal .am-panel { max-height: 30vh; overflow: auto; }
.artifact-modal .am-log { font-family: var(--font-mono); font-size: 12px; white-space: pre-wrap; margin: 0; }
.artifact-modal .am-empty { color: var(--text-faint); font-size: 12.5px; padding: 16px; }
.artifact-modal .am-inputs { display: flex; flex-direction: column; gap: 4px; }
.artifact-modal .am-input { text-align: left; background: var(--bg-sunken); border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 6px 9px; font: inherit; font-size: 12px; font-family: var(--font-mono); color: var(--text-muted); cursor: default; }
.artifact-modal .am-input.linkable { color: var(--clay-strong); cursor: pointer; }
.artifact-modal .am-input.linkable:hover { border-color: var(--clay); }
.artifact-modal .am-env { width: 100%; border-collapse: collapse; font-size: 12px; font-family: var(--font-mono); }
.artifact-modal .am-env td { padding: 3px 8px; border-bottom: 1px solid var(--border); }
.artifact-modal .am-env td:last-child { color: var(--text-faint); text-align: right; }
```

- [ ] **Step 6: Compile the UI.**

Run: `cd ui && cargo check --target wasm32-unknown-unknown`
Expected: clean.

- [ ] **Step 7: Mock the command + write the e2e.** In `ui-tests/tests/mock-tauri.ts`, add a case in the invoke switch:

```ts
        case "get_artifact_provenance":
          return {
            code: "import matplotlib\nplt.savefig('volcano.png')",
            language: "python",
            output: "saved volcano.png",
            exit_status: "ok",
            inputs: [{ path: "DE_results.csv", produced_here: false }],
            env: { name: "kernel", packages: [{ name: "matplotlib", version: "3.8.0" }] },
          };
```

In `ui-tests/tests/ui.spec.ts`, add (adapt the setup lines — opening a project + producing an image artifact — to match the existing "artifacts panel" test at line ~54):

```ts
test("clicking a figure opens the artifact modal with provenance", async ({ page }) => {
  await page.goto("/");
  await page.locator(".proj-card:not(.proj-example)").first().click();
  // Produce a figure into the artifacts panel (mirror the existing artifacts test setup).
  await page.getByPlaceholder(/Ask wisp-science/i).fill("make a volcano plot volcano.png");
  await page.getByRole("button", { name: "Send" }).click();
  await page.locator(".rp-tab", { hasText: "Artifacts" }).click();
  await page.locator(".rp-tile").first().click();
  await page.locator(".rp-view-head .icon-btn[title]").last().click(); // expand
  await expect(page.locator(".artifact-modal")).toBeVisible();
  await page.locator(".am-tab", { hasText: "Code" }).click();
  await expect(page.locator(".artifact-modal")).toContainText("savefig");
  await page.locator(".am-tab", { hasText: "Environment" }).click();
  await expect(page.locator(".am-env")).toContainText("matplotlib");
});
```

- [ ] **Step 8: Run the e2e.**

Run: `cd ui-tests && npx playwright test -g "artifact modal"`
Expected: PASS. (If the mock's artifact-detection path differs, align the setup with the passing `uploaded file shows up in the artifacts panel` test.)

- [ ] **Step 9: Commit.**

```bash
git add ui/src/dto.rs ui/src/main.rs ui/src/i18n.rs ui/src/styles/modals.css ui-tests/tests/mock-tauri.ts ui-tests/tests/ui.spec.ts
git commit -m "feat(ui): ArtifactModal — click figure for Code/Log/Inputs/Environment provenance"
```

---

## Self-Review

**Spec coverage:**
- §3.1 capture (snapshot/diff, producing-tools-only, empty-write skip) → Task 2 ✓
- §3.2 lazy per-session env snapshot → Task 3 (`capture_env`, drain-task first-record guard) ✓
- §3.3 schema (execution_log, env_snapshots; no artifacts coupling) → Task 1 ✓
- §3.4 `get_artifact_provenance` + ArtifactModal (Code/Log/Inputs/Env, empty state, path-linked inputs) → Task 4 + Task 5 ✓
- §5 error handling (best-effort, non-fatal env, None→empty) → Task 2 (swallowed), Task 3 (`unwrap_or`, `tracing::warn`), Task 4 (`None`) ✓
- §6 testing (store roundtrip, snapshot/diff unit, e2e) → Tasks 1, 2, 5 ✓

**Placeholder scan:** Two named-but-reuse helpers in Task 5 (`file_kind_for`, `reveal_in_os`) are flagged with explicit reuse-or-fallback instructions rather than left blank — acceptable, but confirm the existing symbols during implementation.

**Type consistency:** `ExecLog` fields (Task 1) match the drain-task literal (Task 3) and the command's reads (Task 4). `ProvenanceRecord` (Task 2) matches `TauriOutput::provenance` + drain (Task 3). `ArtifactProvenance`/`ProvInput`/`ProvEnv`/`ProvPkg` shapes match between Rust command (Task 4) and UI DTO (Task 5) and the e2e mock (Task 5).
