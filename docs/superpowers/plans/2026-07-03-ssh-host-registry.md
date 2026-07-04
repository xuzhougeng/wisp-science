# SSH Host Registry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Register SSH hosts (from `~/.ssh/config` or typed), persist them, tell the agent they exist, and manage them in a composer button + right-sidebar tab.

**Architecture:** A tauri-layer module (`ssh_hosts.rs`) owns a `SshHost` model, pure list/parse/render helpers, and 4 commands persisting a JSON blob under the `ssh_hosts` store key. The rendered host list is injected as a `## Compute hosts` section in the agent's system prompt at session start. The Leptos UI adds a composer popover, an add-host modal, and a `RightTab::Hosts` tab.

**Tech Stack:** Rust (tauri 2, wisp-core, wisp-store, serde_json, `dirs`), Leptos 0.6 WASM, existing `Store::{get,set}_setting`.

## Global Constraints

- Rust WASM compile-check: `~/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo check --target wasm32-unknown-unknown` from `ui/` (cargo is not on PATH).
- Native build/test: same cargo, `--manifest-path <repo>/Cargo.toml -p wisp-tauri`.
- Package names: backend `wisp-tauri`, frontend `wisp-ui`, core `wisp-core`, store `wisp-store`.
- i18n: every user-facing string needs both `Locale::En` and `Locale::Zh` in `ui/src/i18n.rs` (missing key renders the raw key).
- Do NOT bundle upstream proprietary prompts; host-guidance text is self-authored.
- Store setting key: `ssh_hosts` (JSON array). No new DB table.
- `SshHost` field names must match byte-for-byte between backend (`src-tauri/src/ssh_hosts.rs`) and frontend (`ui/src/main.rs`) for serde to round-trip.

---

### Task 1: `~/.ssh/config` alias parser

**Files:**
- Create: `src-tauri/src/ssh_hosts.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod ssh_hosts;` next to `mod review;`)

**Interfaces:**
- Produces: `pub fn parse_ssh_config_aliases(config: &str) -> Vec<String>`

- [ ] **Step 1: Create the module with a stub + failing test**

```rust
//! SSH host registry: model, pure transforms, and tauri commands. The agent
//! reaches these hosts with its existing `shell` tool (`ssh <alias> '<cmd>'`);
//! this module just tracks which hosts exist and tells the agent about them.

/// Parse `Host` aliases from an ~/.ssh/config body. Skips wildcard patterns
/// (`*`, `?` — those are match rules, not connectable hosts) and dedupes,
/// preserving first-seen order.
pub fn parse_ssh_config_aliases(config: &str) -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_host_aliases_skips_wildcards_and_dedupes() {
        let cfg = "\
Host gpu-box lab-gpu
    HostName 10.0.0.5
    User alice

Host *
    ForwardAgent yes

Host biowulf
    HostName biowulf.nih.gov

Host gpu-box
    Port 2222
";
        assert_eq!(
            parse_ssh_config_aliases(cfg),
            vec!["gpu-box".to_string(), "lab-gpu".to_string(), "biowulf".to_string()]
        );
    }
}
```

Add `mod ssh_hosts;` in `src-tauri/src/lib.rs` immediately after the `mod review;` line.

- [ ] **Step 2: Run the test — verify it fails**

Run: `~/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test --manifest-path /Users/xuzhougeng/Documents/Code/wisp-science/Cargo.toml -p wisp-tauri --lib ssh_hosts::`
Expected: FAIL — `assertion failed`, left is `[]`.

- [ ] **Step 3: Implement the parser**

```rust
pub fn parse_ssh_config_aliases(config: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in config.lines() {
        let line = line.trim();
        let mut parts = line.split_whitespace();
        let Some(kw) = parts.next() else { continue };
        if !kw.eq_ignore_ascii_case("host") {
            continue;
        }
        for alias in parts {
            if alias.contains('*') || alias.contains('?') {
                continue;
            }
            if !out.iter().any(|a| a == alias) {
                out.push(alias.to_string());
            }
        }
    }
    out
}
```

- [ ] **Step 4: Run the test — verify it passes**

Run: same as Step 2.
Expected: PASS (1 test).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ssh_hosts.rs src-tauri/src/lib.rs
git commit -m "feat(ssh): parse ~/.ssh/config host aliases"
```

---

### Task 2: `SshHost` model + upsert/remove transforms

**Files:**
- Modify: `src-tauri/src/ssh_hosts.rs`

**Interfaces:**
- Produces: `pub struct SshHost { alias: String, user: Option<String>, port: Option<u16>, identity_file: Option<String>, notes: Option<String> }`
- Produces: `pub fn upsert_host(hosts: Vec<SshHost>, host: SshHost) -> Vec<SshHost>`
- Produces: `pub fn remove_host(hosts: Vec<SshHost>, alias: &str) -> Vec<SshHost>`

- [ ] **Step 1: Write failing tests**

Add to the top of `ssh_hosts.rs` (imports) and inside `mod tests`:

```rust
// top of file
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SshHost {
    pub alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

pub fn upsert_host(_hosts: Vec<SshHost>, _host: SshHost) -> Vec<SshHost> {
    Vec::new()
}
pub fn remove_host(_hosts: Vec<SshHost>, _alias: &str) -> Vec<SshHost> {
    Vec::new()
}
```

```rust
// inside mod tests
fn host(alias: &str, notes: Option<&str>) -> SshHost {
    SshHost { alias: alias.into(), user: None, port: None, identity_file: None, notes: notes.map(Into::into) }
}

#[test]
fn upsert_adds_new_and_replaces_by_alias_in_place() {
    let list = vec![host("a", Some("first")), host("b", None)];
    let added = upsert_host(list, host("c", None));
    assert_eq!(added.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);

    let replaced = upsert_host(added, host("a", Some("second")));
    assert_eq!(replaced.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
    assert_eq!(replaced[0].notes.as_deref(), Some("second"));
}

#[test]
fn remove_drops_matching_alias() {
    let list = vec![host("a", None), host("b", None)];
    let out = remove_host(list, "a");
    assert_eq!(out.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(), ["b"]);
}
```

- [ ] **Step 2: Run tests — verify they fail**

Run: `... -p wisp-tauri --lib ssh_hosts::`
Expected: FAIL — `upsert_adds_new...` and `remove_drops...` fail (empty vecs).

- [ ] **Step 3: Implement the transforms**

```rust
pub fn upsert_host(mut hosts: Vec<SshHost>, host: SshHost) -> Vec<SshHost> {
    if let Some(existing) = hosts.iter_mut().find(|h| h.alias == host.alias) {
        *existing = host;
    } else {
        hosts.push(host);
    }
    hosts
}
pub fn remove_host(mut hosts: Vec<SshHost>, alias: &str) -> Vec<SshHost> {
    hosts.retain(|h| h.alias != alias);
    hosts
}
```

- [ ] **Step 4: Run tests — verify they pass**

Run: same. Expected: PASS (3 tests total incl. Task 1).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ssh_hosts.rs
git commit -m "feat(ssh): SshHost model + upsert/remove transforms"
```

---

### Task 3: Render the `## Compute hosts` system-prompt section

**Files:**
- Modify: `src-tauri/src/ssh_hosts.rs`

**Interfaces:**
- Produces: `pub fn render_hosts_section(hosts: &[SshHost]) -> Option<String>`

- [ ] **Step 1: Write failing tests**

```rust
// stub above mod tests
pub fn render_hosts_section(_hosts: &[SshHost]) -> Option<String> {
    None
}
```

```rust
// inside mod tests
#[test]
fn render_empty_is_none() {
    assert!(render_hosts_section(&[]).is_none());
}

#[test]
fn render_lists_conn_and_notes() {
    let hosts = vec![
        SshHost { alias: "gpu".into(), user: Some("alice".into()), port: Some(2222), identity_file: None, notes: Some("slurm; sbatch".into()) },
        host("plain", None),
    ];
    let s = render_hosts_section(&hosts).unwrap();
    assert!(s.starts_with("## Compute hosts"), "{s}");
    assert!(s.contains("ssh <alias>"), "must teach the shell invocation:\n{s}");
    assert!(s.contains("alice@gpu:2222"), "conn missing:\n{s}");
    assert!(s.contains("slurm; sbatch"), "notes missing:\n{s}");
    assert!(s.contains("- plain"), "bare alias missing:\n{s}");
}
```

- [ ] **Step 2: Run tests — verify they fail**

Run: `... -p wisp-tauri --lib ssh_hosts::`
Expected: FAIL — `render_lists_conn_and_notes` (None.unwrap panics).

- [ ] **Step 3: Implement the renderer**

```rust
pub fn render_hosts_section(hosts: &[SshHost]) -> Option<String> {
    if hosts.is_empty() {
        return None;
    }
    let mut s = String::from(
        "## Compute hosts\n\n\
The user has these SSH hosts available. Run remote commands with the shell \
tool: `ssh <alias> '<cmd>'`. Prefer them for heavy jobs; remote paths live on \
the host, not on this machine.\n\n",
    );
    for h in hosts {
        let mut conn = String::new();
        if let Some(u) = &h.user {
            conn.push_str(u);
            conn.push('@');
        }
        conn.push_str(&h.alias);
        if let Some(p) = h.port {
            conn.push_str(&format!(":{p}"));
        }
        s.push_str(&format!("- {} — {}", h.alias, conn));
        if let Some(n) = h.notes.as_deref().filter(|n| !n.trim().is_empty()) {
            s.push_str(&format!(" — {n}"));
        }
        s.push('\n');
    }
    Some(s)
}
```

- [ ] **Step 4: Run tests — verify they pass**

Run: same. Expected: PASS (5 tests total).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ssh_hosts.rs
git commit -m "feat(ssh): render Compute hosts system-prompt section"
```

---

### Task 4: Tauri commands + persistence

**Files:**
- Modify: `src-tauri/src/ssh_hosts.rs` (add commands + `load`/`save` helpers)
- Modify: `src-tauri/src/lib.rs` (register 4 commands in `generate_handler![]`)

**Interfaces:**
- Consumes: `AppState.store: wisp_store::Store` (has `get_setting`/`set_setting`); `parse_ssh_config_aliases`, `upsert_host`, `remove_host` (Tasks 1-2).
- Produces (tauri commands, callable from FE via `invoke`):
  - `list_ssh_hosts() -> Vec<SshHost>`
  - `add_ssh_host(host: SshHost) -> Vec<SshHost>`
  - `remove_ssh_host(alias: String) -> Vec<SshHost>`
  - `list_ssh_config_aliases() -> Vec<String>`

- [ ] **Step 1: Add persistence helpers + commands**

At the top of `ssh_hosts.rs` add imports and helpers; append the commands. `AppState` and `Store` come from `crate` — reference them via `crate::AppState`.

```rust
use tauri::State;

const KEY: &str = "ssh_hosts";

async fn load(store: &wisp_store::Store) -> Vec<SshHost> {
    store
        .get_setting(KEY)
        .await
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

async fn save(store: &wisp_store::Store, hosts: &[SshHost]) -> Result<(), String> {
    let json = serde_json::to_string(hosts).map_err(|e| e.to_string())?;
    store.set_setting(KEY, &json).await.map_err(|e| e.to_string())
}

/// Public: read the persisted hosts for system-prompt injection (Task 5).
pub async fn stored_hosts(store: &wisp_store::Store) -> Vec<SshHost> {
    load(store).await
}

#[tauri::command]
pub async fn list_ssh_hosts(state: State<'_, crate::AppState>) -> Result<Vec<SshHost>, String> {
    Ok(load(&state.store).await)
}

#[tauri::command]
pub async fn add_ssh_host(state: State<'_, crate::AppState>, host: SshHost) -> Result<Vec<SshHost>, String> {
    if host.alias.trim().is_empty() {
        return Err("Alias is required.".into());
    }
    let hosts = upsert_host(load(&state.store).await, host);
    save(&state.store, &hosts).await?;
    Ok(hosts)
}

#[tauri::command]
pub async fn remove_ssh_host(state: State<'_, crate::AppState>, alias: String) -> Result<Vec<SshHost>, String> {
    let hosts = remove_host(load(&state.store).await, &alias);
    save(&state.store, &hosts).await?;
    Ok(hosts)
}

#[tauri::command]
pub async fn list_ssh_config_aliases() -> Result<Vec<String>, String> {
    let text = dirs::home_dir()
        .map(|h| h.join(".ssh").join("config"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    Ok(parse_ssh_config_aliases(&text))
}
```

Note: `AppState.store` must be reachable from `ssh_hosts.rs`. Confirm `struct AppState` and its `store` field are visible to sibling modules (they are `pub(crate)` or in the same crate — the field is accessed as `state.store`; if `AppState` fields are private to `lib.rs`, add `pub(crate)` to the `store` field). Verify by compiling.

- [ ] **Step 2: Register the commands**

In `src-tauri/src/lib.rs`, inside `tauri::generate_handler![ ... ]`, add after `review_session,`:

```rust
            review_session,
            ssh_hosts::list_ssh_hosts,
            ssh_hosts::add_ssh_host,
            ssh_hosts::remove_ssh_host,
            ssh_hosts::list_ssh_config_aliases,
```

- [ ] **Step 3: Compile-check the backend**

Run: `~/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo check --manifest-path /Users/xuzhougeng/Documents/Code/wisp-science/Cargo.toml -p wisp-tauri --message-format=short`
Expected: `Finished`, no errors. Fix any `AppState.store` visibility error by marking the field `pub(crate)`.

- [ ] **Step 4: Re-run the unit tests (still green)**

Run: `... -p wisp-tauri --lib ssh_hosts::`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ssh_hosts.rs src-tauri/src/lib.rs
git commit -m "feat(ssh): tauri commands + JSON persistence for host registry"
```

---

### Task 5: Inject hosts into the system prompt

**Files:**
- Modify: `crates/wisp-core/src/system_prompt.rs` (add `compute_hosts` field + param, include in `assemble`)
- Modify: `crates/wisp-core/src/lib.rs` (`seed_system_prompt` signature)
- Modify: `src-tauri/src/lib.rs` (both `seed_system_prompt` call sites: load hosts, render, pass)

**Interfaces:**
- Consumes: `ssh_hosts::stored_hosts(&store)`, `ssh_hosts::render_hosts_section(&[SshHost])` (Tasks 3-4).
- Produces: `SystemPrompt::new(project_root, skills, compute_hosts: Option<String>)`; `Agent::seed_system_prompt(&mut self, skills: &SkillIndex, compute_hosts: Option<String>)`.

- [ ] **Step 1: Failing test for assemble including the section**

Add to `crates/wisp-core/src/system_prompt.rs` a `#[cfg(test)] mod tests` (or extend one):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use wisp_skills::SkillIndex;

    #[test]
    fn assemble_includes_compute_hosts_when_present() {
        let skills = SkillIndex::default();
        let sp = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, Some("## Compute hosts\n\n- gpu — gpu\n".into()));
        let out = sp.assemble();
        assert!(out.contains("## Compute hosts"), "hosts section missing:\n{out}");
    }

    #[test]
    fn assemble_omits_compute_hosts_when_none() {
        let skills = SkillIndex::default();
        let sp = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, None);
        assert!(!sp.assemble().contains("## Compute hosts"));
    }
}
```

If `SkillIndex::default()` does not exist, construct an empty index the same way existing wisp-core tests do (check `crates/wisp-core/src` for an existing `SkillIndex` test constructor and reuse it verbatim).

- [ ] **Step 2: Run — verify it fails to compile (arity mismatch)**

Run: `~/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test --manifest-path /Users/xuzhougeng/Documents/Code/wisp-science/Cargo.toml -p wisp-core --lib system_prompt::`
Expected: FAIL — `SystemPrompt::new` takes 2 args, not 3.

- [ ] **Step 3: Add the field, param, and assemble line**

In `SystemPrompt`:

```rust
pub struct SystemPrompt<'a> {
    project_root: &'a Path,
    skills: &'a SkillIndex,
    user_rules: Option<String>,
    compute_hosts: Option<String>,
}

impl<'a> SystemPrompt<'a> {
    pub fn new(project_root: &'a Path, skills: &'a SkillIndex, compute_hosts: Option<String>) -> Self {
        let user_rules = std::fs::read_to_string(project_root.join(".wisp").join("WISP.md")).ok().filter(|s| !s.trim().is_empty());
        Self { project_root, skills, user_rules, compute_hosts }
    }
```

Change `assemble` to build a Vec so the section is conditional:

```rust
    pub fn assemble(&self) -> String {
        let mut sections = vec![
            Self::base_intro(),
            Self::safety(),
            Self::builtin_rules(),
            Self::tool_guidance(),
            self.skills_guidance(),
        ];
        if let Some(hosts) = &self.compute_hosts {
            sections.push(hosts.clone());
        }
        sections.push(self.memory());
        sections.push(self.environment());
        sections.join("\n\n")
    }
```

- [ ] **Step 4: Update `seed_system_prompt` in `crates/wisp-core/src/lib.rs`**

```rust
    pub fn seed_system_prompt(&mut self, skills: &SkillIndex, compute_hosts: Option<String>) {
        if self.ctx.is_empty() {
            let prompt = SystemPrompt::new(&self.root, skills, compute_hosts).assemble();
            self.ctx.append_system(prompt);
        }
    }
```

- [ ] **Step 5: Update both call sites in `src-tauri/src/lib.rs`**

In `send_message`, the call is currently `agent.seed_system_prompt(&ap.skills);` (inside `if agent.ctx.is_empty()`). Replace with:

```rust
        if agent.ctx.is_empty() {
            let hosts = ssh_hosts::stored_hosts(&state.store).await;
            agent.seed_system_prompt(&ap.skills, ssh_hosts::render_hosts_section(&hosts));
        }
```

In `new_session`, the call is `agent.seed_system_prompt(&ap.skills);`. Replace with the same two lines (load `hosts`, pass `render_hosts_section(&hosts)`). Confirm `state.store` / `ap`/`state` are in scope at that site; if `new_session` builds the agent from a different binding, load hosts from the `Store` it has access to.

- [ ] **Step 6: Run wisp-core tests + backend check**

Run: `... -p wisp-core --lib system_prompt::` → PASS (2 tests).
Run: `... -p wisp-tauri --message-format=short` → `Finished`, no errors.

- [ ] **Step 7: Commit**

```bash
git add crates/wisp-core/src/system_prompt.rs crates/wisp-core/src/lib.rs src-tauri/src/lib.rs
git commit -m "feat(ssh): inject Compute hosts section into the system prompt"
```

---

### Task 6: Frontend model, signals, and data load

**Files:**
- Modify: `ui/src/main.rs` (add `SshHost` struct, signals, mount-time load)

**Interfaces:**
- Consumes: tauri commands `list_ssh_hosts`, `list_ssh_config_aliases`, `add_ssh_host`, `remove_ssh_host` (Task 4) via the existing `invoke` JS binding.
- Produces: `ssh_hosts: RwSignal<Vec<SshHost>>`, `show_add_host: RwSignal<bool>`, `config_aliases: RwSignal<Vec<String>>` (used by Tasks 7-9).

- [ ] **Step 1: Add the frontend model (mirror the backend fields exactly)**

Near the other `#[derive(...)]` structs (e.g. by `ArtifactInfo`) in `ui/src/main.rs`:

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct SshHost {
    alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    identity_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
}
```

- [ ] **Step 2: Add signals + load on mount**

In the main component body (near other `create_rw_signal` calls such as `ctx_menu`):

```rust
    let ssh_hosts = create_rw_signal::<Vec<SshHost>>(vec![]);
    let show_add_host = create_rw_signal(false);
    let config_aliases = create_rw_signal::<Vec<String>>(vec![]);

    // Load persisted hosts once at startup.
    {
        let ssh_hosts = ssh_hosts;
        spawn_local(async move {
            let v = invoke("list_ssh_hosts", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<SshHost>>(v) {
                ssh_hosts.set(list);
            }
        });
    }
```

- [ ] **Step 3: Compile-check (wasm)**

Run (from `ui/`): `~/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo check --target wasm32-unknown-unknown --message-format=short`
Expected: `Finished`. (Unused `show_add_host`/`config_aliases` warnings are fine until Task 7/8 wire them; if the build denies warnings, prefix with `let _ = ...` temporarily — but check first, wisp does not deny warnings.)

- [ ] **Step 4: Commit**

```bash
git add ui/src/main.rs
git commit -m "feat(ssh): frontend SshHost model + load hosts on startup"
```

---

### Task 7: Composer compute button + popover + i18n + CSS

**Files:**
- Modify: `ui/src/main.rs` (button in `.composer-tools`, popover)
- Modify: `ui/src/i18n.rs` (en + zh strings)
- Modify: `ui/styles.css` (`.composer-compute`, `.compute-menu`)

**Interfaces:**
- Consumes: `ssh_hosts`, `show_add_host`, `config_aliases`, `right_tab`, `show_right` signals; `RightTab::Hosts` (added in Task 9 — this task references it, so do Task 9's enum-variant addition first OR add the variant here).
- Produces: a compute popover with "Add SSH host…", the host list, and "Manage".

- [ ] **Step 1: Add i18n strings** (`ui/src/i18n.rs`, both locales)

En block (near `composer.*`):

```rust
        (Locale::En, "compute.button") => Some("Compute hosts"),
        (Locale::En, "compute.add_host") => Some("Add SSH host…"),
        (Locale::En, "compute.manage") => Some("Manage hosts"),
        (Locale::En, "compute.none") => Some("No hosts yet"),
        (Locale::En, "hosts.title") => Some("Compute hosts"),
        (Locale::En, "hosts.add") => Some("Add SSH host"),
        (Locale::En, "hosts.remove") => Some("Remove"),
        (Locale::En, "hosts.alias") => Some("Host alias"),
        (Locale::En, "hosts.from_config") => Some("From ~/.ssh/config"),
        (Locale::En, "hosts.pick") => Some("Pick a host…"),
        (Locale::En, "hosts.or_type") => Some("Or type a host alias"),
        (Locale::En, "hosts.notes") => Some("Anything the agent should know? (optional)"),
        (Locale::En, "hosts.notes_ph") => Some("How do jobs run — sbatch, qsub, or bash? conda ok? which partition/module?"),
        (Locale::En, "hosts.advanced") => Some("Advanced (override ~/.ssh/config)"),
        (Locale::En, "hosts.user") => Some("User"),
        (Locale::En, "hosts.port") => Some("Port"),
        (Locale::En, "hosts.identity") => Some("Identity file"),
        (Locale::En, "hosts.cancel") => Some("Cancel"),
        (Locale::En, "hosts.save") => Some("Add"),
        (Locale::En, "hosts.empty") => Some("No SSH hosts registered. Add one so the agent can use it."),
```

Zh block (near `composer.*`):

```rust
        (Locale::Zh, "compute.button") => Some("计算主机"),
        (Locale::Zh, "compute.add_host") => Some("添加 SSH 主机…"),
        (Locale::Zh, "compute.manage") => Some("管理主机"),
        (Locale::Zh, "compute.none") => Some("暂无主机"),
        (Locale::Zh, "hosts.title") => Some("计算主机"),
        (Locale::Zh, "hosts.add") => Some("添加 SSH 主机"),
        (Locale::Zh, "hosts.remove") => Some("移除"),
        (Locale::Zh, "hosts.alias") => Some("主机别名"),
        (Locale::Zh, "hosts.from_config") => Some("从 ~/.ssh/config 选择"),
        (Locale::Zh, "hosts.pick") => Some("选择一个主机…"),
        (Locale::Zh, "hosts.or_type") => Some("或输入主机别名"),
        (Locale::Zh, "hosts.notes") => Some("有什么需要 agent 知道的？（可选）"),
        (Locale::Zh, "hosts.notes_ph") => Some("作业怎么跑——sbatch、qsub 还是 bash？能用 conda 吗？哪个分区/module？"),
        (Locale::Zh, "hosts.advanced") => Some("高级（覆盖 ~/.ssh/config）"),
        (Locale::Zh, "hosts.user") => Some("用户"),
        (Locale::Zh, "hosts.port") => Some("端口"),
        (Locale::Zh, "hosts.identity") => Some("密钥文件"),
        (Locale::Zh, "hosts.cancel") => Some("取消"),
        (Locale::Zh, "hosts.save") => Some("添加"),
        (Locale::Zh, "hosts.empty") => Some("还没有 SSH 主机。添加一个，agent 就能使用它。"),
```

- [ ] **Step 2: Add the button + popover in `.composer-tools`**

In `ui/src/main.rs`, inside `<div class="composer-tools">` after the existing `.composer-plus` button block and its compose-menu, add a local signal near the composer signals: `let compute_menu_open = create_rw_signal(false);` Then insert:

```rust
                            <button type="button" class="composer-compute"
                                class:active=move || compute_menu_open.get()
                                title=move || t(locale.get(), "compute.button")
                                on:click=move |_| compute_menu_open.update(|o| *o = !*o)>
                                {compose_icon("server")}
                            </button>
                            {move || compute_menu_open.get().then(|| view! {
                                <div class="compose-backdrop" on:click=move |_| compute_menu_open.set(false)></div>
                                <div class="compose-menu compute-menu">
                                    <button type="button" class="compose-item" on:click=move |_| {
                                        compute_menu_open.set(false);
                                        show_add_host.set(true);
                                        spawn_local(async move {
                                            let v = invoke("list_ssh_config_aliases", JsValue::UNDEFINED).await;
                                            if let Ok(a) = serde_wasm_bindgen::from_value::<Vec<String>>(v) { config_aliases.set(a); }
                                        });
                                    }>
                                        <span class="compose-item-icon">{compose_icon("server")}</span>
                                        <span class="compose-item-text">
                                            <span class="compose-item-label">{move || t(locale.get(), "compute.add_host")}</span>
                                        </span>
                                    </button>
                                    <div class="compose-group">
                                        <div class="compose-group-label">{move || t(locale.get(), "hosts.title")}</div>
                                        {move || {
                                            let hs = ssh_hosts.get();
                                            if hs.is_empty() {
                                                view! { <div class="compose-item-sub" style="padding:6px 18px">{move || t(locale.get(), "compute.none")}</div> }.into_view()
                                            } else {
                                                hs.into_iter().map(|h| view! {
                                                    <button type="button" class="compose-item" on:click=move |_| {
                                                        compute_menu_open.set(false); right_tab.set(RightTab::Hosts); show_right.set(true);
                                                    }>
                                                        <span class="compose-item-icon">{compose_icon("server")}</span>
                                                        <span class="compose-item-text"><span class="compose-item-label">{h.alias.clone()}</span></span>
                                                    </button>
                                                }.into_view()).collect_view()
                                            }
                                        }}
                                    </div>
                                </div>
                            })}
```

- [ ] **Step 3: Add a `server` icon arm to `compose_icon`**

In `ui/src/main.rs`, in the `compose_icon` match, add before the `_` arm:

```rust
        "server" => view! { <rect x="3" y="4" width="18" height="7" rx="1"/><rect x="3" y="13" width="18" height="7" rx="1"/><circle cx="7" cy="7.5" r="0.5" fill="currentColor"/><circle cx="7" cy="16.5" r="0.5" fill="currentColor"/> }.into_view(),
```

- [ ] **Step 4: Add CSS** (`ui/styles.css`, after `.composer-plus` rules)

```css
.composer-compute { display: inline-flex; align-items: center; justify-content: center; width: 32px; height: 32px; flex: 0 0 auto; border-radius: 999px; border: 1px solid hsl(48 12% 86%); background: hsl(48 20% 98%); color: var(--text-muted); cursor: pointer; transition: background .12s ease, color .12s ease, border-color .12s ease; }
.composer-compute:hover, .composer-compute.active { background: var(--bg-sunken); color: var(--text); border-color: hsl(48 10% 78%); }
.composer-compute svg { width: 17px; height: 17px; }
.compute-menu { width: 280px; }
```

- [ ] **Step 5: Compile-check (wasm)** — this will error until `RightTab::Hosts` exists (Task 9). Add the enum variant now:

In `ui/src/main.rs` change `enum RightTab { Artifacts, File, Provenance }` to `enum RightTab { Artifacts, File, Provenance, Hosts }`. The `match right_tab.get()` render block will now be non-exhaustive — add a temporary `RightTab::Hosts => view!{}.into_view(),` arm (Task 9 fills it).

Run (from `ui/`): `cargo check --target wasm32-unknown-unknown --message-format=short` → `Finished`.

- [ ] **Step 6: Commit**

```bash
git add ui/src/main.rs ui/src/i18n.rs ui/styles.css
git commit -m "feat(ssh): composer compute button + host popover"
```

---

### Task 8: Add-SSH-host modal

**Files:**
- Modify: `ui/src/main.rs` (modal view + form signals + submit)
- Modify: `ui/styles.css` (reuse existing `.modal`/`.drawer` classes; add `.host-form` if needed)

**Interfaces:**
- Consumes: `show_add_host`, `config_aliases`, `ssh_hosts` signals; `add_ssh_host` command.
- Produces: a working add flow that appends to `ssh_hosts` and closes.

- [ ] **Step 1: Add form field signals** (near `show_add_host`)

```rust
    let host_alias = create_rw_signal(String::new());
    let host_user = create_rw_signal(String::new());
    let host_port = create_rw_signal(String::new());
    let host_identity = create_rw_signal(String::new());
    let host_notes = create_rw_signal(String::new());
```

- [ ] **Step 2: Add the modal view** (render near other modals like the settings/demos modal; gate on `show_add_host`)

```rust
        {move || show_add_host.get().then(|| view! {
            <div class="modal-backdrop" on:click=move |_| show_add_host.set(false)></div>
            <div class="modal host-modal">
                <h2>{move || t(locale.get(), "hosts.add")}</h2>
                <label class="host-label">{move || t(locale.get(), "hosts.from_config")}</label>
                <select class="host-input" on:change=move |ev| host_alias.set(event_target_value(&ev))>
                    <option value="">{move || t(locale.get(), "hosts.pick")}</option>
                    {move || config_aliases.get().into_iter().map(|a| view! { <option value=a.clone()>{a}</option> }).collect_view()}
                </select>
                <label class="host-label">{move || t(locale.get(), "hosts.or_type")}</label>
                <input class="host-input" prop:value=move || host_alias.get() on:input=move |ev| host_alias.set(event_target_value(&ev)) />
                <label class="host-label">{move || t(locale.get(), "hosts.notes")}</label>
                <textarea class="host-input" prop:value=move || host_notes.get()
                    placeholder=move || t(locale.get(), "hosts.notes_ph")
                    on:input=move |ev| host_notes.set(event_target_value(&ev))></textarea>
                <details class="host-advanced">
                    <summary>{move || t(locale.get(), "hosts.advanced")}</summary>
                    <label class="host-label">{move || t(locale.get(), "hosts.user")}</label>
                    <input class="host-input" prop:value=move || host_user.get() on:input=move |ev| host_user.set(event_target_value(&ev)) />
                    <label class="host-label">{move || t(locale.get(), "hosts.port")}</label>
                    <input class="host-input" prop:value=move || host_port.get() on:input=move |ev| host_port.set(event_target_value(&ev)) />
                    <label class="host-label">{move || t(locale.get(), "hosts.identity")}</label>
                    <input class="host-input" prop:value=move || host_identity.get() on:input=move |ev| host_identity.set(event_target_value(&ev)) />
                </details>
                <div class="host-actions">
                    <button class="btn" on:click=move |_| show_add_host.set(false)>{move || t(locale.get(), "hosts.cancel")}</button>
                    <button class="btn primary" disabled=move || host_alias.get().trim().is_empty()
                        on:click=move |_| {
                            let opt = |s: String| { let s = s.trim().to_string(); if s.is_empty() { None } else { Some(s) } };
                            let host = SshHost {
                                alias: host_alias.get().trim().to_string(),
                                user: opt(host_user.get()),
                                port: host_port.get().trim().parse::<u16>().ok(),
                                identity_file: opt(host_identity.get()),
                                notes: opt(host_notes.get()),
                            };
                            let arg = serde_wasm_bindgen::to_value(&serde_json::json!({ "host": host })).unwrap();
                            spawn_local(async move {
                                let v = invoke("add_ssh_host", arg).await;
                                if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<SshHost>>(v) { ssh_hosts.set(list); }
                            });
                            host_alias.set(String::new()); host_user.set(String::new()); host_port.set(String::new());
                            host_identity.set(String::new()); host_notes.set(String::new());
                            show_add_host.set(false);
                        }>{move || t(locale.get(), "hosts.save")}</button>
                </div>
            </div>
        })}
```

Note: check the exact `invoke` binding arity in `ui/src/api.js` / the `extern` block — pass args as a `JsValue` object with a `host` key matching the command's parameter name (`host`). If wisp's `invoke` wraps args differently (e.g. `{ "args": {...} }`), match the existing pattern used by `send_message`/`create_project`.

- [ ] **Step 3: Add modal CSS** (`ui/styles.css`)

```css
.host-modal { max-width: 460px; }
.host-label { display:block; font-size:12px; color:var(--text-muted); margin:12px 0 4px; }
.host-input { width:100%; padding:8px 10px; border:1px solid var(--border-strong); border-radius:var(--radius-sm); background:var(--bg-input); color:var(--text); font:inherit; font-size:13.5px; }
textarea.host-input { min-height:64px; resize:vertical; }
.host-advanced { margin-top:12px; border-top:1px solid var(--border); padding-top:8px; }
.host-advanced summary { cursor:pointer; font-size:13px; color:var(--text-muted); }
.host-actions { display:flex; justify-content:flex-end; gap:8px; margin-top:16px; }
```

Reuse existing `.modal`, `.modal-backdrop`, `.btn`, `.btn.primary` classes — confirm they exist in `styles.css` (they back the settings modal). If class names differ, match the settings modal's markup.

- [ ] **Step 4: Compile-check (wasm)** → `Finished`.

- [ ] **Step 5: Commit**

```bash
git add ui/src/main.rs ui/styles.css
git commit -m "feat(ssh): add-SSH-host modal (config dropdown + advanced + notes)"
```

---

### Task 9: Right-sidebar Hosts tab

**Files:**
- Modify: `ui/src/main.rs` (tab button + Hosts render arm)

**Interfaces:**
- Consumes: `ssh_hosts`, `right_tab`, `show_add_host` signals; `remove_ssh_host` command; `RightTab::Hosts` (added in Task 7 Step 5).
- Produces: a sidebar tab listing hosts with remove + add.

- [ ] **Step 1: Add the tab button** — in the `.rp-tabs` row (next to the Artifacts/File/Provenance `<button class="rp-tab">`s):

```rust
                    <button class="rp-tab" class:active=move || right_tab.get() == RightTab::Hosts
                        on:click=move |_| right_tab.set(RightTab::Hosts)>
                        {move || t(locale.get(), "hosts.title")}
                    </button>
```

- [ ] **Step 2: Replace the temporary `RightTab::Hosts` arm** (added in Task 7) with the real render:

```rust
                        RightTab::Hosts => {
                            let loc = locale.get();
                            let hs = ssh_hosts.get();
                            view! {
                                <div class="rp-hosts">
                                    <button type="button" class="rp-empty-action" style="margin:10px"
                                        on:click=move |_| {
                                            show_add_host.set(true);
                                            spawn_local(async move {
                                                let v = invoke("list_ssh_config_aliases", JsValue::UNDEFINED).await;
                                                if let Ok(a) = serde_wasm_bindgen::from_value::<Vec<String>>(v) { config_aliases.set(a); }
                                            });
                                        }>{t(loc, "hosts.add")}</button>
                                    {if hs.is_empty() {
                                        view! { <div class="rp-empty"><div class="rp-empty-title">{t(loc, "hosts.empty")}</div></div> }.into_view()
                                    } else {
                                        hs.into_iter().map(|h| {
                                            let alias = h.alias.clone();
                                            let conn = {
                                                let mut c = String::new();
                                                if let Some(u) = &h.user { c.push_str(u); c.push('@'); }
                                                c.push_str(&h.alias);
                                                if let Some(p) = h.port { c.push_str(&format!(":{p}")); }
                                                c
                                            };
                                            view! {
                                                <div class="host-card">
                                                    <div class="host-card-head">
                                                        <span class="host-card-alias">{h.alias.clone()}</span>
                                                        <button type="button" class="host-card-remove"
                                                            on:click=move |_| {
                                                                let alias = alias.clone();
                                                                let arg = serde_wasm_bindgen::to_value(&serde_json::json!({ "alias": alias })).unwrap();
                                                                spawn_local(async move {
                                                                    let v = invoke("remove_ssh_host", arg).await;
                                                                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<SshHost>>(v) { ssh_hosts.set(list); }
                                                                });
                                                            }>"×"</button>
                                                    </div>
                                                    <div class="host-card-conn">{conn}</div>
                                                    {h.notes.clone().map(|n| view! { <div class="host-card-notes">{n}</div> })}
                                                </div>
                                            }
                                        }).collect_view()
                                    }}
                                </div>
                            }.into_view()
                        }
```

- [ ] **Step 3: Add CSS** (`ui/styles.css`)

```css
.rp-hosts { display:flex; flex-direction:column; gap:8px; padding:4px 10px 12px; }
.host-card { border:1px solid var(--border-strong); border-radius:var(--radius-sm); padding:10px 12px; background:var(--bg-elev); }
.host-card-head { display:flex; align-items:center; justify-content:space-between; }
.host-card-alias { font-weight:600; font-size:14px; }
.host-card-remove { border:none; background:transparent; color:var(--text-faint); cursor:pointer; font-size:16px; line-height:1; }
.host-card-remove:hover { color:var(--err); }
.host-card-conn { font-family:var(--font-mono); font-size:12px; color:var(--text-muted); margin-top:4px; }
.host-card-notes { font-size:12.5px; color:var(--text-muted); margin-top:6px; white-space:pre-wrap; }
```

- [ ] **Step 4: Compile-check (wasm)** → `Finished`.

- [ ] **Step 5: Commit**

```bash
git add ui/src/main.rs ui/styles.css
git commit -m "feat(ssh): right-sidebar Hosts tab with add/remove"
```

---

### Task 10: End-to-end verification + preview

**Files:** none (verification only); optionally sync `ui/preview.html` with a sample hosts tab for the visual-iteration mock.

- [ ] **Step 1: Full backend gate**

Run: `... test --manifest-path <repo>/Cargo.toml -p wisp-tauri --lib ssh_hosts::` → PASS (5).
Run: `... test --manifest-path <repo>/Cargo.toml -p wisp-core --lib system_prompt::` → PASS (2).
Run: `... check --manifest-path <repo>/Cargo.toml -p wisp-tauri` → `Finished`.

- [ ] **Step 2: Full frontend gate**

Run (from `ui/`): `cargo check --target wasm32-unknown-unknown` → `Finished`, no new warnings.

- [ ] **Step 3: Preview the UI** (mock in `preview.html` or the real app if runnable)

Add a sample add-host modal + Hosts tab card to `ui/preview.html`, start the `ui-preview` server, and screenshot to confirm the compute button, popover, modal, and sidebar card render in the teal theme.

- [ ] **Step 4: Manual smoke (if the tauri app runs locally)** — open compute menu → Add SSH host → pick/type alias + notes → Add → confirm it appears in the sidebar and persists across restart; start a session and confirm the agent's context contains the `## Compute hosts` section.

- [ ] **Step 5: Final commit + push + PR** (see execution handoff)

```bash
git push -u origin feat/ssh-host-registry
```

---

## Self-Review

**Spec coverage:**
- Data model + `ssh_hosts` persistence → Tasks 2, 4. ✓
- 4 backend commands + config parser → Tasks 1, 4. ✓
- `## Compute hosts` injection at seed time → Task 5. ✓
- Composer button + popover → Task 7. ✓
- Add-host panel (config dropdown / manual / advanced / notes) → Task 8. ✓
- Right-sidebar Hosts tab → Task 9. ✓
- Non-goals (terminal/tmux/run_on_host) → not implemented, correct. ✓
- Testing (parser TDD, upsert/remove TDD, render TDD, compile+preview) → Tasks 1-3, 10. ✓

**Type consistency:** `SshHost { alias, user, port: Option<u16>, identity_file, notes }` identical in `src-tauri/src/ssh_hosts.rs` (Task 2) and `ui/src/main.rs` (Task 6). `seed_system_prompt(&SkillIndex, Option<String>)` defined in Task 5 and called in Task 5 Step 5. `render_hosts_section(&[SshHost]) -> Option<String>` produced Task 3, consumed Task 5. `RightTab::Hosts` added Task 7 Step 5, rendered Task 9. Command names (`list_ssh_hosts`, `add_ssh_host`, `remove_ssh_host`, `list_ssh_config_aliases`) consistent across Tasks 4, 6, 7, 8, 9.

**Placeholder scan:** No TBD/TODO; every code step shows full code. Two explicit "verify the existing pattern" notes (AppState.store visibility in Task 4; `invoke` arg-wrapping convention in Task 8) are real integration checks, not placeholders — each says exactly what to confirm and the fallback.
