# Settings Page: Skill Management + MCP Connections — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a multi-section Settings page (General / Skills / Connections) that lets users enable/disable and add skills, and configure local (stdio) or remote (HTTP) MCP servers.

**Architecture:** Persistence reuses the existing SQLite key-value store (three new JSON keys, no schema change). A new HTTP transport is added to `wisp-mcp` behind the existing `McpClient` API. Skill filtering and MCP wiring happen at agent-creation time, so toggles apply to new sessions; idle cached agents are reset (reusing the pattern already in `set_settings`) so changes land on the next turn. The Leptos Settings modal becomes a left-nav shell hosting the three sections.

**Tech Stack:** Rust, Tauri v2, Leptos 0.6 (CSR→WASM, Trunk), SQLite (`wisp-store`), `reqwest` 0.12 (already a workspace dep, has `stream`+`json`+`rustls-tls`), `tauri-plugin-dialog` v2 (already installed).

## Global Constraints

- MCP transport values are serde-tagged: `#[serde(tag = "kind", rename_all = "lowercase")]` → JSON `"kind": "stdio"` / `"kind": "http"`.
- Settings JSON keys (exact): `disabled_skills`, `mcp_connections`, `bio_tools_enabled`.
- User-writable skill dir: `~/.wisp/skills/<name>/` via `dirs::home_dir().join(".wisp").join("skills")`. Bundled skills live under `wisp_skills::bundled_dir()` and may be disabled but never removed.
- Public `wisp-mcp` API (`McpClient::tools_list`, `McpClient::tool_call`, `McpTool`) MUST remain unchanged — `McpTool` holds `Arc<McpClient>` and must not need edits.
- Toggle/skill changes take effect on new sessions (skills) / next turn for idle sessions (MCP). Do NOT attempt mid-conversation hot-swap.
- No new heavy dependencies. `.zip` skill upload and GitHub import are explicitly out of scope.
- Rust package names for commands: `wisp-mcp`, `wisp-skills`, `wisp-tauri` (the Tauri backend crate in `src-tauri/`), `wisp-ui` (Leptos frontend in `ui/`).
- Follow existing patterns: `#[tauri::command]` fns registered in the `generate_handler!` block (lib.rs:1430); frontend uses `invoke`/`invoke_checked` (ui/src/api.js) and the `t(locale, key)` i18n helper.

---

## File Structure

**Backend crates:**
- `crates/wisp-mcp/Cargo.toml` — add `reqwest` dep.
- `crates/wisp-mcp/src/client.rs` — `Transport` enum (Stdio/Http), `connect_http`, SSE parsing. Public API unchanged.
- `crates/wisp-mcp/src/lib.rs` — re-export nothing new (connect_http is a `McpClient` method).
- `crates/wisp-skills/src/index.rs` — `SkillIndex::filtered`.

**Tauri backend:**
- `src-tauri/src/lib.rs` — new types (`McpConnection`, `McpTransport`, extended `SkillInfo`), settings load/save helpers, skill install/remove, connection CRUD + test commands, `wire_python_and_mcp` extension, agent-creation filter, `generate_handler!` registration.

**Frontend:**
- `ui/src/main.rs` — Settings modal → left-nav shell with General/Skills/Connections sections + signals + invoke calls.
- `ui/src/i18n.rs` — new translation keys (en + zh).
- `ui/styles.css` — settings-nav layout + list-row + toggle styles.

---

## Phase A — wisp-mcp HTTP transport

### Task A1: Refactor `McpClient` to a transport enum (no behavior change)

**Files:**
- Modify: `crates/wisp-mcp/src/client.rs`
- Modify: `crates/wisp-mcp/Cargo.toml`

**Interfaces:**
- Consumes: nothing new.
- Produces: `McpClient` with a private `transport: Transport` field; public methods `launch`, `launch_bio_tools`, `tools_list`, `tool_call` unchanged. New private `enum Transport { Stdio { stdin, stdout, next_id }, Http(HttpTransport) }` (HttpTransport added in A2). `McpClient::request(&self, method, params)` dispatches on `transport`.

- [ ] **Step 1: Add reqwest to wisp-mcp Cargo.toml**

In `crates/wisp-mcp/Cargo.toml` under `[dependencies]`, add:

```toml
reqwest = { workspace = true }
```

- [ ] **Step 2: Refactor client.rs to move stdio state behind `Transport::Stdio`**

Replace the `McpClient` struct and its stdio internals so the three stdio fields live in a `Transport::Stdio` variant. Keep everything else identical.

```rust
enum Transport {
    Stdio {
        stdin: Arc<Mutex<ChildStdin>>,
        stdout: Arc<Mutex<BufReader<ChildStdout>>>,
        next_id: AtomicU64,
    },
    // Http variant added in Task A2
}

pub struct McpClient {
    transport: Transport,
}

impl McpClient {
    pub async fn launch(command: &str, args: &[String]) -> Result<Self> {
        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args);
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        wisp_tools::process::hide_console_async(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| anyhow!("spawn MCP server '{command}': {e}"))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        std::mem::forget(child);

        let client = Self {
            transport: Transport::Stdio {
                stdin: Arc::new(Mutex::new(stdin)),
                stdout: Arc::new(Mutex::new(BufReader::new(stdout))),
                next_id: AtomicU64::new(1),
            },
        };
        let init_params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "wisp-science", "version": env!("CARGO_PKG_VERSION") }
        });
        let _ = client.request("initialize", Some(init_params)).await?;
        client.notify("notifications/initialized", json!({})).await?;
        Ok(client)
    }

    async fn request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        match &self.transport {
            Transport::Stdio { stdin, stdout, next_id } => {
                let id = next_id.fetch_add(1, Ordering::SeqCst);
                let req = JsonRpcReq { jsonrpc: "2.0", id, method: method.to_string(), params };
                let val = serde_json::to_value(&req)?;
                // send
                {
                    let mut w = stdin.lock().await;
                    w.write_all(val.to_string().as_bytes()).await?;
                    w.write_all(b"\n").await?;
                    w.flush().await?;
                }
                // read matching id
                loop {
                    let mut line = String::new();
                    let mut r = stdout.lock().await;
                    let n = r.read_line(&mut line).await?;
                    drop(r);
                    if n == 0 { return Err(anyhow!("MCP server closed stdout")); }
                    let trimmed = line.trim();
                    if trimmed.is_empty() { continue; }
                    let resp: JsonRpcResp = serde_json::from_str(trimmed)?;
                    if resp.id == Some(id) {
                        if let Some(e) = resp.error { return Err(anyhow!("MCP error: {}", e.message)); }
                        return Ok(resp.result.unwrap_or(Value::Null));
                    }
                }
            }
        }
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        match &self.transport {
            Transport::Stdio { stdin, .. } => {
                let val = json!({ "jsonrpc": "2.0", "method": method, "params": params });
                let mut w = stdin.lock().await;
                w.write_all(val.to_string().as_bytes()).await?;
                w.write_all(b"\n").await?;
                w.flush().await?;
                Ok(())
            }
        }
    }
}
```

Delete the now-unused `send_raw` and `read_response` methods (their logic moved inline into `request`). Keep `tools_list`, `tool_call`, `launch_bio_tools`, `toals_into_remote`, `RemoteTool`, `JsonRpcReq/Resp/Error` as-is.

> Note: the `match` has one arm now; the compiler will warn "non-exhaustive" is not an issue since it's exhaustive with one variant. The Http arm is added in A2.

- [ ] **Step 3: Verify it compiles and existing behavior is intact**

Run: `cargo build -p wisp-mcp`
Expected: builds with no errors (warnings about unused imports are fine to clean up).

- [ ] **Step 4: Commit**

```bash
git add crates/wisp-mcp/Cargo.toml crates/wisp-mcp/src/client.rs
git commit -m "refactor(mcp): move stdio state behind a Transport enum"
```

---

### Task A2: Add Streamable-HTTP transport + SSE parsing

**Files:**
- Modify: `crates/wisp-mcp/src/client.rs`
- Test: inline `#[cfg(test)] mod tests` in `crates/wisp-mcp/src/client.rs`

**Interfaces:**
- Consumes: `Transport` enum from A1.
- Produces:
  - `pub async fn McpClient::connect_http(url: &str, headers: &[(String, String)]) -> Result<Self>` — POSTs `initialize`, stores session id + headers.
  - `fn parse_jsonrpc_from_sse(body: &str, expected_id: u64) -> Result<Value>` — extracts the JSON-RPC result whose `id` matches from an SSE `text/event-stream` body.
  - `Transport::Http(HttpTransport)` where `struct HttpTransport { client: reqwest::Client, url: String, headers: Vec<(String,String)>, session_id: tokio::sync::Mutex<Option<String>>, next_id: AtomicU64 }`.

- [ ] **Step 1: Write the failing test for SSE parsing**

Add to the bottom of `crates/wisp-mcp/src/client.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_body_yields_matching_jsonrpc_result() {
        // An MCP server may answer over text/event-stream. Frames are
        // `data: <json>` lines separated by blank lines. We want the result
        // whose id == expected_id, ignoring unrelated notifications.
        let body = "event: message\n\
                    data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{}}\n\
                    \n\
                    event: message\n\
                    data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"tools\":[]}}\n\
                    \n";
        let got = parse_jsonrpc_from_sse(body, 7).unwrap();
        assert_eq!(got, serde_json::json!({ "tools": [] }));
    }

    #[test]
    fn sse_body_surfaces_jsonrpc_error() {
        let body = "data: {\"jsonrpc\":\"2.0\",\"id\":3,\"error\":{\"message\":\"boom\"}}\n\n";
        let err = parse_jsonrpc_from_sse(body, 3).unwrap_err();
        assert!(err.to_string().contains("boom"));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p wisp-mcp sse_body`
Expected: FAIL — `parse_jsonrpc_from_sse` not found.

- [ ] **Step 3: Implement the HTTP transport and SSE parser**

Add near the top of `client.rs` (after the existing `use` lines):

```rust
use std::sync::atomic::AtomicU64; // already imported; keep single import
```

Add the `Http` variant to `Transport`:

```rust
enum Transport {
    Stdio { /* unchanged */ },
    Http(HttpTransport),
}

struct HttpTransport {
    client: reqwest::Client,
    url: String,
    headers: Vec<(String, String)>,
    session_id: tokio::sync::Mutex<Option<String>>,
    next_id: AtomicU64,
}
```

Add the parser (free function so it is unit-testable without a live server):

```rust
/// Pull the JSON-RPC response with `expected_id` out of a `text/event-stream`
/// body. Each SSE frame carries one JSON object on a `data:` line; we scan
/// every data line and return the first whose id matches.
fn parse_jsonrpc_from_sse(body: &str, expected_id: u64) -> Result<Value> {
    for line in body.lines() {
        let line = line.trim_start();
        let Some(data) = line.strip_prefix("data:") else { continue };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" { continue; }
        let Ok(resp) = serde_json::from_str::<JsonRpcResp>(data) else { continue };
        if resp.id == Some(expected_id) {
            if let Some(e) = resp.error { return Err(anyhow!("MCP error: {}", e.message)); }
            return Ok(resp.result.unwrap_or(Value::Null));
        }
    }
    Err(anyhow!("no JSON-RPC response for id {expected_id} in SSE stream"))
}
```

Add `connect_http` and extend `request`/`notify` with the Http arm:

```rust
impl McpClient {
    pub async fn connect_http(url: &str, headers: &[(String, String)]) -> Result<Self> {
        let client = Self {
            transport: Transport::Http(HttpTransport {
                client: reqwest::Client::new(),
                url: url.to_string(),
                headers: headers.to_vec(),
                session_id: tokio::sync::Mutex::new(None),
                next_id: AtomicU64::new(1),
            }),
        };
        let init_params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "wisp-science", "version": env!("CARGO_PKG_VERSION") }
        });
        let _ = client.request("initialize", Some(init_params)).await?;
        client.notify("notifications/initialized", json!({})).await?;
        Ok(client)
    }
}
```

In `request`, add the Http arm:

```rust
Transport::Http(h) => {
    let id = h.next_id.fetch_add(1, Ordering::SeqCst);
    let req = JsonRpcReq { jsonrpc: "2.0", id, method: method.to_string(), params };
    let mut rb = h.client
        .post(&h.url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .json(&req);
    if let Some(sid) = h.session_id.lock().await.clone() {
        rb = rb.header("mcp-session-id", sid);
    }
    for (k, v) in &h.headers { rb = rb.header(k.as_str(), v.as_str()); }
    let resp = rb.send().await.map_err(|e| anyhow!("http mcp request: {e}"))?;
    if let Some(sid) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
        *h.session_id.lock().await = Some(sid.to_string());
    }
    let ctype = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let status = resp.status();
    let text = resp.text().await.map_err(|e| anyhow!("http mcp body: {e}"))?;
    if !status.is_success() {
        return Err(anyhow!("http mcp {status}: {}", text.chars().take(200).collect::<String>()));
    }
    if ctype.contains("text/event-stream") {
        parse_jsonrpc_from_sse(&text, id)
    } else {
        let resp: JsonRpcResp = serde_json::from_str(text.trim())?;
        if let Some(e) = resp.error { return Err(anyhow!("MCP error: {}", e.message)); }
        Ok(resp.result.unwrap_or(Value::Null))
    }
}
```

In `notify`, add the Http arm (notifications are fire-and-forget POSTs; a 202 with no body is normal):

```rust
Transport::Http(h) => {
    let val = json!({ "jsonrpc": "2.0", "method": method, "params": params });
    let mut rb = h.client
        .post(&h.url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .json(&val);
    if let Some(sid) = h.session_id.lock().await.clone() {
        rb = rb.header("mcp-session-id", sid);
    }
    for (k, v) in &h.headers { rb = rb.header(k.as_str(), v.as_str()); }
    let _ = rb.send().await.map_err(|e| anyhow!("http mcp notify: {e}"))?;
    Ok(())
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p wisp-mcp`
Expected: PASS (both `sse_body_*` tests) and the crate builds.

- [ ] **Step 5: Commit**

```bash
git add crates/wisp-mcp/src/client.rs
git commit -m "feat(mcp): add Streamable-HTTP transport with SSE response parsing"
```

---

## Phase B — Skills backend

### Task B1: `SkillIndex::filtered`

**Files:**
- Modify: `crates/wisp-skills/src/index.rs`
- Test: inline `#[cfg(test)] mod tests` in `crates/wisp-skills/src/index.rs`

**Interfaces:**
- Produces: `pub fn SkillIndex::filtered(&self, disabled: &std::collections::HashSet<String>) -> SkillIndex` — returns a new index containing only skills whose `name` is NOT in `disabled`. Both `descriptions()` and `get()` on the returned index then exclude disabled skills.

- [ ] **Step 1: Write the failing test**

Add to the bottom of `crates/wisp-skills/src/index.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn skill(name: &str) -> Skill {
        Skill { name: name.into(), description: format!("desc {name}"), tags: vec![], body: String::new(), dir: PathBuf::new() }
    }

    #[test]
    fn filtered_drops_disabled_skills() {
        let idx = SkillIndex { skills: vec![skill("a"), skill("b"), skill("c")] };
        let disabled: HashSet<String> = ["b".to_string()].into_iter().collect();
        let out = idx.filtered(&disabled);
        let names: Vec<_> = out.all().iter().map(|s| s.name.clone()).collect();
        assert_eq!(names, vec!["a", "c"]);
        assert!(out.get("b").is_none());
        assert!(out.get("a").is_some());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p wisp-skills filtered_drops_disabled`
Expected: FAIL — no method named `filtered`.

- [ ] **Step 3: Implement `filtered`**

Add to `impl SkillIndex` (after `find`):

```rust
    /// A new index without any skill whose name is in `disabled`.
    pub fn filtered(&self, disabled: &std::collections::HashSet<String>) -> SkillIndex {
        SkillIndex {
            skills: self.skills.iter().filter(|s| !disabled.contains(&s.name)).cloned().collect(),
        }
    }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p wisp-skills filtered_drops_disabled`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/wisp-skills/src/index.rs
git commit -m "feat(skills): add SkillIndex::filtered to exclude disabled skills"
```

---

### Task B2: Persist `disabled_skills` + apply the filter at agent creation

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Test: extend `#[cfg(test)] mod tests` in `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `SkillIndex::filtered` (B1); `Store::get_setting`/`set_setting`.
- Produces:
  - `async fn load_disabled_skills(store: &Store) -> std::collections::HashSet<String>` — parses the `disabled_skills` JSON array (empty set if missing/invalid).
  - `async fn save_disabled_skills(store: &Store, set: &std::collections::HashSet<String>) -> Result<(), String>`.
  - `fn parse_disabled_skills(raw: Option<&str>) -> HashSet<String>` — pure helper (testable).
  - Agent-creation path in `send_message` filters `ap.skills` by the disabled set before `Agent::new`/`seed_system_prompt`.

- [ ] **Step 1: Write the failing test for the pure parser**

In the `mod tests` block at the bottom of `src-tauri/src/lib.rs`, add:

```rust
    use super::parse_disabled_skills;

    #[test]
    fn parse_disabled_skills_handles_missing_and_valid() {
        assert!(parse_disabled_skills(None).is_empty());
        assert!(parse_disabled_skills(Some("not json")).is_empty());
        let s = parse_disabled_skills(Some(r#"["alphafold2","boltz"]"#));
        assert!(s.contains("alphafold2") && s.contains("boltz") && s.len() == 2);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p wisp-tauri parse_disabled_skills`
Expected: FAIL — `parse_disabled_skills` not found.

- [ ] **Step 3: Implement the helpers**

Add near the other settings helpers (e.g. after `load_settings`, ~lib.rs:410):

```rust
use std::collections::HashSet;

fn parse_disabled_skills(raw: Option<&str>) -> HashSet<String> {
    raw.and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .map(|v| v.into_iter().collect())
        .unwrap_or_default()
}

async fn load_disabled_skills(store: &Store) -> HashSet<String> {
    let raw = store.get_setting("disabled_skills").await.ok().flatten();
    parse_disabled_skills(raw.as_deref())
}

async fn save_disabled_skills(store: &Store, set: &HashSet<String>) -> Result<(), String> {
    let mut v: Vec<&String> = set.iter().collect();
    v.sort();
    let json = serde_json::to_string(&v).map_err(|e| format!("{e}"))?;
    store.set_setting("disabled_skills", &json).await.map_err(|e| format!("{e}"))
}
```

- [ ] **Step 4: Apply the filter at agent creation**

In `send_message`, replace the agent-build block (lib.rs ~571-582). The change: load the disabled set, build a filtered `Arc<SkillIndex>`, use it for both `Agent::new` and `seed_system_prompt`.

```rust
    let mut guard = rt.agent.lock().await;
    if guard.is_none() {
        let disabled = load_disabled_skills(&state.store).await;
        let skills = Arc::new(ap.skills.filtered(&disabled));
        let mut agent = Agent::new(cfg.clone(), skills.clone(), ap.memory.clone(), ap.root.clone(), max_context, max_iter);
        match state.store.load_messages(&frame_id).await {
            Ok(msgs) => agent.ctx.messages = msgs,
            Err(e) => tracing::warn!("load session from sqlite failed: {e}"),
        }
        rt.set_last_seq(agent.ctx.messages.len() as i64);
        if agent.ctx.is_empty() {
            let hosts = ssh_hosts::stored_hosts(&state.store).await;
            agent.seed_system_prompt(&skills, ssh_hosts::render_hosts_section(&hosts));
        }
        let wire_errors = wire_python_and_mcp(&mut agent, &state.app_data, &state.store).await;
        if !wire_errors.is_empty() {
            state.bootstrap.lock().unwrap().errors.extend(wire_errors);
        }
        *guard = Some(agent);
    }
```

> Note: `wire_python_and_mcp` gains a `&state.store` argument — implemented in Task C2. Until C2 lands, temporarily call it with the old 2-arg signature and add the store arg in C2. To keep this task self-contained and compiling, leave the call as `wire_python_and_mcp(&mut agent, &state.app_data).await` here and change BOTH the call and the fn signature together in C2.

**Correction for this task:** keep the existing 2-arg call unchanged in B2 (only the skills-filter lines change). The 3-arg form is introduced in C2.

- [ ] **Step 5: Run tests + build**

Run: `cargo test -p wisp-tauri parse_disabled_skills && cargo build -p wisp-tauri`
Expected: test PASS, crate builds.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(settings): persist disabled_skills and filter them at agent creation"
```

---

### Task B3: Skill commands — list (extended), toggle, install, remove

**Files:**
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `load_disabled_skills`/`save_disabled_skills` (B2); `SkillIndex::load`; `skill_paths` (lib.rs:445); `tauri-plugin-dialog`.
- Produces (all `#[tauri::command]`, registered in `generate_handler!`):
  - Extended `struct SkillInfo { name, description, enabled: bool, builtin: bool, dir: String }`.
  - `async fn list_skills(state) -> Vec<SkillInfo>` (now async: reads disabled set).
  - `async fn set_skill_enabled(state, name: String, enabled: bool) -> Result<(), String>`.
  - `async fn pick_skill_source(app) -> Result<Option<String>, String>` — dialog picks a folder OR a SKILL.md file.
  - `async fn install_skill(state, src_path: String) -> Result<String, String>` — copies into `~/.wisp/skills/<name>/`, returns the installed name; reloads `ActiveProject.skills`.
  - `async fn remove_skill(state, name: String) -> Result<(), String>` — deletes a user skill dir only; reloads.

- [ ] **Step 1: Extend the `SkillInfo` struct**

Replace the struct at lib.rs:44-48:

```rust
#[derive(Serialize, Clone)]
struct SkillInfo {
    name: String,
    description: String,
    enabled: bool,
    builtin: bool,
    dir: String,
}
```

- [ ] **Step 2: Rewrite `list_skills` (async, with status)**

Replace `list_skills` (lib.rs:877-881). A skill is `builtin` when its dir is under `wisp_skills::bundled_dir()`:

```rust
#[tauri::command]
async fn list_skills(state: State<'_, AppState>) -> Result<Vec<SkillInfo>, String> {
    let ap = state.active();
    let disabled = load_disabled_skills(&state.store).await;
    let bundled = wisp_skills::bundled_dir();
    Ok(ap.skills.all().iter().map(|s| {
        let builtin = bundled.as_ref().map(|b| s.dir.starts_with(b)).unwrap_or(false);
        SkillInfo {
            name: s.name.clone(),
            description: s.description.clone(),
            enabled: !disabled.contains(&s.name),
            builtin,
            dir: s.dir.to_string_lossy().to_string(),
        }
    }).collect())
}
```

> `get_capabilities` also builds `Vec<SkillInfo>` (lib.rs ~1189). Update that construction to fill the three new fields (set `enabled`/`builtin` the same way, or `enabled: true, builtin: true, dir: …` if simpler for the read-only Capabilities view). Verify `cargo build` after.

- [ ] **Step 3: Add `set_skill_enabled`**

```rust
#[tauri::command]
async fn set_skill_enabled(state: State<'_, AppState>, name: String, enabled: bool) -> Result<(), String> {
    let mut set = load_disabled_skills(&state.store).await;
    if enabled { set.remove(&name); } else { set.insert(name); }
    save_disabled_skills(&state.store, &set).await?;
    reset_idle_agents(&state).await;
    Ok(())
}
```

Add the shared reset helper (extract the loop already inlined in `set_settings` lib.rs:963-968, then call it from `set_settings` too):

```rust
/// Drop idle cached agents so the next turn rebuilds with fresh config.
/// A running turn holds its agent mutex and keeps the old config until it ends.
async fn reset_idle_agents(state: &AppState) {
    let runtimes = state.sessions.lock().await.values().cloned().collect::<Vec<_>>();
    for rt in runtimes {
        if let Ok(mut guard) = rt.agent.try_lock() { *guard = None; }
    }
}
```

- [ ] **Step 4: Add `pick_skill_source` + `install_skill` + `remove_skill`**

```rust
#[tauri::command]
async fn pick_skill_source(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    // Let the user pick a SKILL.md; folder picking is offered via a second button
    // in the UI that calls pick_directory (existing command).
    app.dialog().file().add_filter("SKILL.md", &["md"]).pick_file(move |p| { let _ = tx.send(p); });
    let picked = rx.await.map_err(|e| format!("{e}"))?;
    Ok(picked.map(|fp| fp.to_string()))
}

fn user_skills_dir() -> Result<PathBuf, String> {
    dirs::home_dir().map(|h| h.join(".wisp").join("skills"))
        .ok_or_else(|| "no home directory".to_string())
}

#[tauri::command]
async fn install_skill(state: State<'_, AppState>, src_path: String) -> Result<String, String> {
    let src = PathBuf::from(&src_path);
    // Resolve the skill's source dir + the SKILL.md path.
    let (skill_dir, skill_md) = if src.is_dir() {
        let md = src.join("SKILL.md");
        if !md.is_file() { return Err("selected folder has no SKILL.md".into()); }
        (src.clone(), md)
    } else if src.file_name().map(|n| n == "SKILL.md").unwrap_or(false) {
        (src.parent().map(PathBuf::from).unwrap_or_default(), src.clone())
    } else {
        return Err("select a skill folder or a SKILL.md file".into());
    };
    // Parse name from frontmatter (fall back to dir name), validate description.
    let skill = wisp_skills::parse_skill_file(&skill_md)
        .ok_or_else(|| "could not parse SKILL.md frontmatter".to_string())?;
    if skill.description.trim().is_empty() {
        return Err("SKILL.md is missing a description".into());
    }
    let dest = user_skills_dir()?.join(&skill.name);
    if dest.exists() { return Err(format!("a skill named '{}' already exists", skill.name)); }
    std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| format!("{e}"))?;
    copy_dir_recursive(&skill_dir, &dest).map_err(|e| format!("{e}"))?;
    reload_skills(&state);
    reset_idle_agents(&state).await;
    Ok(skill.name)
}

#[tauri::command]
async fn remove_skill(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let dir = user_skills_dir()?.join(&name);
    if !dir.is_dir() { return Err("only user-added skills can be removed".into()); }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("{e}"))?;
    // Also drop it from the disabled set so a re-add starts clean.
    let mut set = load_disabled_skills(&state.store).await;
    set.remove(&name);
    let _ = save_disabled_skills(&state.store, &set).await;
    reload_skills(&state);
    reset_idle_agents(&state).await;
    Ok(())
}

fn reload_skills(state: &AppState) {
    let root = state.active().root;
    let skills = Arc::new(SkillIndex::load(&skill_paths(&root)));
    state.active.write().unwrap().skills = skills;
}

fn copy_dir_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}
```

> `install_skill` uses `wisp_skills::parse_skill_file` — the existing `parse_skill` in `crates/wisp-skills/src/index.rs` is private and takes `(path, dir)`. Add a thin public wrapper in that crate:
> ```rust
> pub fn parse_skill_file(md: &Path) -> Option<Skill> {
>     let dir = md.parent().map(PathBuf::from).unwrap_or_default();
>     parse_skill(md, dir)
> }
> ```
> Commit that wrapper as part of this task.

- [ ] **Step 5: Register the new commands**

In the `generate_handler!` block (lib.rs:1430), add: `set_skill_enabled, pick_skill_source, install_skill, remove_skill`. (`list_skills` is already listed — its signature changed to async+Result, no registration change needed.)

- [ ] **Step 6: Build**

Run: `cargo build -p wisp-tauri`
Expected: builds. Fix any call sites of `list_skills`/`SkillInfo` the compiler flags (e.g. `get_capabilities`).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/lib.rs crates/wisp-skills/src/index.rs
git commit -m "feat(skills): list status + enable/disable + install/remove commands"
```

---

## Phase C — Connections backend

### Task C1: `McpConnection` model + persistence helpers

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Test: `#[cfg(test)] mod tests` in `src-tauri/src/lib.rs`

**Interfaces:**
- Produces:
  - `#[derive(Serialize, Deserialize, Clone)] struct McpConnection { id: String, name: String, enabled: bool, transport: McpTransport }`.
  - `#[serde(tag = "kind", rename_all = "lowercase")] enum McpTransport { Stdio { command: String, #[serde(default)] args: Vec<String>, #[serde(default)] env: Vec<(String,String)>, #[serde(default)] cwd: Option<String> }, Http { url: String, #[serde(default)] headers: Vec<(String,String)> } }`.
  - `async fn load_mcp_connections(store) -> Vec<McpConnection>` / `async fn save_mcp_connections(store, &[McpConnection]) -> Result<(),String>`.
  - `async fn load_bio_tools_enabled(store) -> bool` (default `true`) / `async fn save_bio_tools_enabled(store, bool)`.

- [ ] **Step 1: Write the failing serde roundtrip test**

In `mod tests` at the bottom of `src-tauri/src/lib.rs`:

```rust
    use super::{McpConnection, McpTransport};

    #[test]
    fn mcp_connection_serde_roundtrip() {
        let stdio = McpConnection {
            id: "1".into(), name: "local".into(), enabled: true,
            transport: McpTransport::Stdio { command: "python".into(), args: vec!["s.py".into()], env: vec![("K".into(),"V".into())], cwd: None },
        };
        let http = McpConnection {
            id: "2".into(), name: "remote".into(), enabled: false,
            transport: McpTransport::Http { url: "https://x/mcp".into(), headers: vec![("Authorization".into(),"Bearer t".into())] },
        };
        for c in [stdio, http] {
            let json = serde_json::to_string(&c).unwrap();
            let back: McpConnection = serde_json::from_str(&json).unwrap();
            assert_eq!(serde_json::to_string(&back).unwrap(), json);
        }
        // tag shape
        let j = serde_json::to_value(&McpConnection { id:"3".into(), name:"n".into(), enabled:true, transport: McpTransport::Http{ url:"u".into(), headers: vec![] } }).unwrap();
        assert_eq!(j["transport"]["kind"], "http");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p wisp-tauri mcp_connection_serde`
Expected: FAIL — types not found.

- [ ] **Step 3: Implement the types + helpers**

Add the types near the other structs (~lib.rs:90). Derive `PartialEq` is not required; `assert_eq` above compares re-serialized JSON strings.

```rust
#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum McpTransport {
    Stdio {
        command: String,
        #[serde(default)] args: Vec<String>,
        #[serde(default)] env: Vec<(String, String)>,
        #[serde(default)] cwd: Option<String>,
    },
    Http {
        url: String,
        #[serde(default)] headers: Vec<(String, String)>,
    },
}

#[derive(Serialize, Deserialize, Clone)]
struct McpConnection {
    id: String,
    name: String,
    enabled: bool,
    transport: McpTransport,
}
```

Add the persistence helpers near the skill helpers:

```rust
async fn load_mcp_connections(store: &Store) -> Vec<McpConnection> {
    store.get_setting("mcp_connections").await.ok().flatten()
        .and_then(|s| serde_json::from_str::<Vec<McpConnection>>(&s).ok())
        .unwrap_or_default()
}

async fn save_mcp_connections(store: &Store, conns: &[McpConnection]) -> Result<(), String> {
    let json = serde_json::to_string(conns).map_err(|e| format!("{e}"))?;
    store.set_setting("mcp_connections", &json).await.map_err(|e| format!("{e}"))
}

async fn load_bio_tools_enabled(store: &Store) -> bool {
    store.get_setting("bio_tools_enabled").await.ok().flatten()
        .and_then(|s| serde_json::from_str::<bool>(&s).ok())
        .unwrap_or(true)
}

async fn save_bio_tools_enabled(store: &Store, on: bool) -> Result<(), String> {
    store.set_setting("bio_tools_enabled", &on.to_string()).await.map_err(|e| format!("{e}"))
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p wisp-tauri mcp_connection_serde`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(connections): McpConnection model + settings persistence"
```

---

### Task C2: Wire connections into the agent + CRUD/test commands

**Files:**
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `load_mcp_connections`/`save_*` (C1); `load_bio_tools_enabled`; `McpClient::launch`/`connect_http`/`tools_list` (A1/A2); `register_mcp` (lib.rs:511); `reset_idle_agents` (B3).
- Produces (commands registered in `generate_handler!`):
  - `wire_python_and_mcp(agent, app_data, store)` — 3-arg; gates bio-tools on `bio_tools_enabled`, then wires each enabled user connection.
  - `struct McpConnectionsView { bio_tools_enabled: bool, connections: Vec<McpConnection> }`.
  - `async fn list_mcp_connections(state) -> McpConnectionsView`.
  - `async fn add_mcp_connection(state, conn)` / `update_mcp_connection(state, conn)` / `delete_mcp_connection(state, id)`.
  - `async fn set_mcp_connection_enabled(state, id, enabled)` / `set_bio_tools_enabled(state, enabled)`.
  - `async fn test_mcp_connection(state, conn) -> Result<usize, String>` — connects, `tools_list`, returns tool count.

- [ ] **Step 1: Add a helper that builds an `McpClient` from a connection**

```rust
async fn connect_mcp(conn: &McpConnection, py_python: Option<&std::path::Path>) -> anyhow::Result<wisp_mcp::McpClient> {
    match &conn.transport {
        McpTransport::Stdio { command, args, env, cwd } => {
            // env/cwd support: set via std::env for the spawned process is unsafe globally;
            // instead prepend `env`-style not needed — wisp_mcp::launch takes command+args.
            // For env/cwd we build the command ourselves.
            let mut cmd = tokio::process::Command::new(command);
            cmd.args(args);
            for (k, v) in env { cmd.env(k, v); }
            if let Some(dir) = cwd { if !dir.is_empty() { cmd.current_dir(dir); } }
            let _ = py_python; // reserved; user stdio connections use their own command
            wisp_mcp::McpClient::launch_with_command(cmd).await
        }
        McpTransport::Http { url, headers } => {
            wisp_mcp::McpClient::connect_http(url, headers).await
        }
    }
}
```

> This needs a `McpClient::launch_with_command(cmd: tokio::process::Command)` constructor in `wisp-mcp` (the current `launch` builds its own `Command` and can't carry env/cwd). Add it in `crates/wisp-mcp/src/client.rs` by extracting the body of `launch` to take a pre-built `Command`:
> ```rust
> pub async fn launch_with_command(mut cmd: tokio::process::Command) -> Result<Self> {
>     cmd.stdin(std::process::Stdio::piped())
>         .stdout(std::process::Stdio::piped())
>         .stderr(std::process::Stdio::null());
>     wisp_tools::process::hide_console_async(&mut cmd);
>     let mut child = cmd.spawn().map_err(|e| anyhow!("spawn MCP server: {e}"))?;
>     let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
>     let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
>     std::mem::forget(child);
>     let client = Self { transport: Transport::Stdio {
>         stdin: Arc::new(Mutex::new(stdin)),
>         stdout: Arc::new(Mutex::new(BufReader::new(stdout))),
>         next_id: AtomicU64::new(1),
>     }};
>     let init_params = json!({ "protocolVersion": "2024-11-05", "capabilities": {},
>         "clientInfo": { "name": "wisp-science", "version": env!("CARGO_PKG_VERSION") } });
>     let _ = client.request("initialize", Some(init_params)).await?;
>     client.notify("notifications/initialized", json!({})).await?;
>     Ok(client)
> }
> ```
> Then make `launch` delegate: build the `Command` and call `launch_with_command`. Commit this wisp-mcp change with this task.

- [ ] **Step 2: Extend `wire_python_and_mcp` to 3 args and wire connections**

Change the signature (lib.rs:457) and the bio-tools block:

```rust
async fn wire_python_and_mcp(agent: &mut wisp_core::Agent, app_data: &std::path::Path, store: &Store) -> Vec<String> {
    let mut errors = vec![];
    let py_env = match wisp_python::PythonEnv::ensure(app_data) {
        Ok(env) => Some(env),
        Err(e) => { errors.push(format!("Python environment: {e}")); None }
    };

    // ... existing Python REPL wiring unchanged ...

    // Bundled bio-tools (gated by the settings toggle, default on).
    if load_bio_tools_enabled(store).await {
        if let Ok(cmdline) = std::env::var("WISP_MCP_COMMAND") {
            // ... existing WISP_MCP_COMMAND branch unchanged ...
        } else if let Some(env) = &py_env {
            let pkg = std::env::var("WISP_MCP_PKG").unwrap_or_else(|_| "mcp_bio".into());
            match wisp_mcp::McpClient::launch_bio_tools(&env.python(), &pkg).await {
                Ok(client) => register_mcp(agent, std::sync::Arc::new(client)).await,
                Err(e) => errors.push(format!("MCP {pkg}: {e}")),
            }
        }
    }

    // User-configured connections.
    for conn in load_mcp_connections(store).await.into_iter().filter(|c| c.enabled) {
        match connect_mcp(&conn, py_env.as_ref().map(|e| e.python()).as_deref()).await {
            Ok(client) => register_mcp(agent, std::sync::Arc::new(client)).await,
            Err(e) => errors.push(format!("MCP '{}': {e}", conn.name)),
        }
    }
    errors
}
```

Update the call site in `send_message` (from B2) to `wire_python_and_mcp(&mut agent, &state.app_data, &state.store).await`.

> `env.python()` returns a `PathBuf`; `.as_deref()` on `Option<PathBuf>` needs `.as_ref().map(|e| e.python())` to be a temporary — bind it to a `let` if the borrow checker complains:
> ```rust
> let py = py_env.as_ref().map(|e| e.python());
> for conn in ... { match connect_mcp(&conn, py.as_deref()).await { ... } }
> ```

- [ ] **Step 3: Add the CRUD + test commands**

```rust
#[derive(Serialize, Clone)]
struct McpConnectionsView {
    bio_tools_enabled: bool,
    connections: Vec<McpConnection>,
}

#[tauri::command]
async fn list_mcp_connections(state: State<'_, AppState>) -> Result<McpConnectionsView, String> {
    Ok(McpConnectionsView {
        bio_tools_enabled: load_bio_tools_enabled(&state.store).await,
        connections: load_mcp_connections(&state.store).await,
    })
}

#[tauri::command]
async fn add_mcp_connection(state: State<'_, AppState>, conn: McpConnection) -> Result<(), String> {
    let mut conns = load_mcp_connections(&state.store).await;
    conns.push(conn);
    save_mcp_connections(&state.store, &conns).await?;
    reset_idle_agents(&state).await;
    Ok(())
}

#[tauri::command]
async fn update_mcp_connection(state: State<'_, AppState>, conn: McpConnection) -> Result<(), String> {
    let mut conns = load_mcp_connections(&state.store).await;
    match conns.iter_mut().find(|c| c.id == conn.id) {
        Some(slot) => *slot = conn,
        None => return Err("connection not found".into()),
    }
    save_mcp_connections(&state.store, &conns).await?;
    reset_idle_agents(&state).await;
    Ok(())
}

#[tauri::command]
async fn delete_mcp_connection(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut conns = load_mcp_connections(&state.store).await;
    conns.retain(|c| c.id != id);
    save_mcp_connections(&state.store, &conns).await?;
    reset_idle_agents(&state).await;
    Ok(())
}

#[tauri::command]
async fn set_mcp_connection_enabled(state: State<'_, AppState>, id: String, enabled: bool) -> Result<(), String> {
    let mut conns = load_mcp_connections(&state.store).await;
    if let Some(c) = conns.iter_mut().find(|c| c.id == id) { c.enabled = enabled; }
    save_mcp_connections(&state.store, &conns).await?;
    reset_idle_agents(&state).await;
    Ok(())
}

#[tauri::command]
async fn set_bio_tools_enabled(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    save_bio_tools_enabled(&state.store, enabled).await?;
    reset_idle_agents(&state).await;
    Ok(())
}

#[tauri::command]
async fn test_mcp_connection(state: State<'_, AppState>, conn: McpConnection) -> Result<usize, String> {
    let py = wisp_python::PythonEnv::ensure(&state.app_data).ok().map(|e| e.python());
    let client = connect_mcp(&conn, py.as_deref()).await.map_err(|e| format!("{e}"))?;
    let tools = client.tools_list().await.map_err(|e| format!("{e}"))?;
    Ok(tools.len())
}
```

- [ ] **Step 4: Register the commands**

In `generate_handler!` add: `list_mcp_connections, add_mcp_connection, update_mcp_connection, delete_mcp_connection, set_mcp_connection_enabled, set_bio_tools_enabled, test_mcp_connection`.

- [ ] **Step 5: Build the whole backend**

Run: `cargo build -p wisp-tauri && cargo test -p wisp-tauri && cargo test -p wisp-mcp`
Expected: builds; all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/lib.rs crates/wisp-mcp/src/client.rs
git commit -m "feat(connections): wire user MCP connections + CRUD/test commands"
```

---

## Phase D — Frontend multi-section Settings page

> The frontend is Leptos CSR (WASM). These tasks verify by building the UI bundle and by manual preview. Build command: `cd ui && trunk build` (Trunk is the bundler; the app runs under `cargo tauri dev`). There is no unit-test harness for the `view!` macros — correctness is confirmed by a clean `trunk build` plus the manual checks listed.

### Task D1: Convert the Settings modal into a left-nav shell (General section)

**Files:**
- Modify: `ui/src/main.rs`
- Modify: `ui/src/i18n.rs`
- Modify: `ui/styles.css`

**Interfaces:**
- Consumes: existing `show_settings`, `settings`, `locale`, `settings_busy`, `save_settings`, `validate_settings`, `check_updates` signals/handlers.
- Produces: a `settings_section: RwSignal<&'static str>` ("general" | "skills" | "connections") driving which pane renders; the existing fields move under the "general" pane.

- [ ] **Step 1: Add the section signal**

Near the other settings signals (ui/src/main.rs ~1642):

```rust
let settings_section = create_rw_signal("general");
```

- [ ] **Step 2: Restructure the modal markup**

Replace the Settings modal body (ui/src/main.rs:3117-3184) so the modal has a left nav + right content. Wrap the modal in `class="modal settings-modal"`. The nav lists three buttons; each sets `settings_section`. The existing General fields (language, provider, api_url, model, api_key, workspace_dir, hint, status, action row) move verbatim inside `{move || (settings_section.get() == "general").then(|| view!{ ... })}`.

```rust
{move || show_settings.get().then(|| view! {
    <div class="overlay">
        <div class="modal settings-modal">
            <div class="settings-nav">
                <button class:active=move || settings_section.get()=="general"
                    on:click=move |_| settings_section.set("general")>
                    {move || t(locale.get(), "settings.nav.general")}</button>
                <button class:active=move || settings_section.get()=="skills"
                    on:click=move |_| settings_section.set("skills")>
                    {move || t(locale.get(), "settings.nav.skills")}</button>
                <button class:active=move || settings_section.get()=="connections"
                    on:click=move |_| settings_section.set("connections")>
                    {move || t(locale.get(), "settings.nav.connections")}</button>
            </div>
            <div class="settings-content">
                <h2>{move || t(locale.get(), "settings.title")}</h2>
                {move || (settings_section.get() == "general").then(|| view! {
                    // ... the existing language/provider/api_url/model/api_key/workspace_dir
                    //     labels + hint + settings_message + action row, moved here verbatim ...
                }.into_view())}
                // Skills pane → Task D2. Connections pane → Task D3.
            </div>
        </div>
    </div>
}.into_view())}
```

- [ ] **Step 3: Add i18n keys**

In `ui/src/i18n.rs`, add (both `en` and `zh` tables):

```
settings.nav.general      → "General" / "常规"
settings.nav.skills       → "Skills"  / "技能"
settings.nav.connections  → "Connections" / "连接"
settings.applies_new_session → "Changes apply to new sessions." / "改动对新会话生效。"
```

- [ ] **Step 4: Add layout CSS**

In `ui/styles.css`:

```css
.settings-modal { display: flex; gap: 0; min-width: 640px; max-width: 820px; padding: 0; }
.settings-nav { display: flex; flex-direction: column; gap: 2px; padding: 16px 8px; border-right: 1px solid var(--border, #e5e5e5); min-width: 150px; }
.settings-nav button { text-align: left; padding: 8px 12px; border: 0; background: transparent; border-radius: 6px; cursor: pointer; }
.settings-nav button.active { background: var(--surface-2, #eef1f0); font-weight: 600; }
.settings-content { flex: 1; padding: 16px 20px; overflow-y: auto; max-height: 70vh; }
```

- [ ] **Step 5: Build + manual check**

Run: `cd ui && trunk build`
Expected: builds. Manual (under `cargo tauri dev`): open Settings → left nav shows General/Skills/Connections; General has the old fields; Save/Validate/Cancel still work.

- [ ] **Step 6: Commit**

```bash
git add ui/src/main.rs ui/src/i18n.rs ui/styles.css
git commit -m "feat(ui): settings left-nav shell with General section"
```

---

### Task D2: Skills section (list + toggle + add + remove)

**Files:**
- Modify: `ui/src/main.rs`
- Modify: `ui/src/i18n.rs`
- Modify: `ui/styles.css`

**Interfaces:**
- Consumes: backend `list_skills` (returns `[{name, description, enabled, builtin, dir}]`), `set_skill_enabled`, `pick_skill_source`, `pick_directory` (existing), `install_skill`, `remove_skill`.
- Produces: a `skills_list: RwSignal<Vec<SkillRow>>` refreshed on open + after mutations; the "skills" pane markup.

- [ ] **Step 1: Add a UI-side row type + signal**

Near the top of `ui/src/main.rs` where other UI structs live, add:

```rust
#[derive(Clone, serde::Deserialize)]
struct SkillRow { name: String, description: String, enabled: bool, builtin: bool, #[allow(dead_code)] dir: String }
```

Near settings signals: `let skills_list = create_rw_signal(Vec::<SkillRow>::new());`

- [ ] **Step 2: Add a refresh action**

```rust
let refresh_skills = move || {
    spawn_local(async move {
        let v = invoke("list_skills", JsValue::UNDEFINED).await;
        if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<SkillRow>>(v) {
            skills_list.set(rows);
        }
    });
};
```

Call `refresh_skills()` when the Settings modal opens (in the existing `open_settings`/`show_settings.set(true)` handler ~ui/src/main.rs:2057), and when the user switches to the skills section.

- [ ] **Step 3: Render the skills pane**

Inside `.settings-content`, after the general pane:

```rust
{move || (settings_section.get() == "skills").then(|| view! {
    <div class="settings-pane">
        <div class="pane-head">
            <span class="hint">{move || t(locale.get(), "settings.applies_new_session")}</span>
            <div class="row">
                <button on:click=move |_| { /* pick SKILL.md */
                    spawn_local(async move {
                        let picked = invoke("pick_skill_source", JsValue::UNDEFINED).await;
                        if let Some(path) = picked.as_string() {
                            let arg = to_value(&serde_json::json!({ "srcPath": path })).unwrap();
                            let _ = invoke_checked("install_skill", arg).await;
                            refresh_skills();
                        }
                    });
                }>{move || t(locale.get(), "skills.add_file")}</button>
                <button on:click=move |_| { /* pick folder */
                    spawn_local(async move {
                        let picked = invoke("pick_directory", JsValue::UNDEFINED).await;
                        if let Some(path) = picked.as_string() {
                            let arg = to_value(&serde_json::json!({ "srcPath": path })).unwrap();
                            let _ = invoke_checked("install_skill", arg).await;
                            refresh_skills();
                        }
                    });
                }>{move || t(locale.get(), "skills.add_folder")}</button>
            </div>
        </div>
        <div class="list">
            <For each=move || skills_list.get() key=|s| s.name.clone() let:s>
                {
                    let name_toggle = s.name.clone();
                    let name_remove = s.name.clone();
                    let enabled = s.enabled;
                    let builtin = s.builtin;
                    view! {
                        <div class="list-row">
                            <div class="list-row-main">
                                <div class="list-row-title">{s.name.clone()}</div>
                                <div class="list-row-sub">{s.description.clone()}</div>
                            </div>
                            {(!builtin).then(|| { let n = name_remove.clone(); view! {
                                <button class="icon-btn" title="remove" on:click=move |_| {
                                    let n = n.clone();
                                    spawn_local(async move {
                                        let arg = to_value(&serde_json::json!({ "name": n })).unwrap();
                                        let _ = invoke_checked("remove_skill", arg).await;
                                        refresh_skills();
                                    });
                                }>"🗑"</button>
                            }})}
                            <input type="checkbox" prop:checked=enabled on:change=move |ev| {
                                let n = name_toggle.clone();
                                let on = event_target_checked(&ev);
                                spawn_local(async move {
                                    let arg = to_value(&serde_json::json!({ "name": n, "enabled": on })).unwrap();
                                    let _ = invoke_checked("set_skill_enabled", arg).await;
                                });
                            } />
                        </div>
                    }
                }
            </For>
        </div>
    </div>
}.into_view())}
```

> `event_target_checked` helper: if not already present, add near `event_target_input`:
> ```rust
> fn event_target_checked(ev: &web_sys::Event) -> bool {
>     ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.checked()).unwrap_or(false)
> }
> ```

- [ ] **Step 4: i18n + CSS**

i18n keys (en/zh):

```
skills.add_file   → "Add SKILL.md" / "添加 SKILL.md"
skills.add_folder → "Add folder"   / "添加文件夹"
```

CSS:

```css
.settings-pane .pane-head { display: flex; justify-content: space-between; align-items: center; margin-bottom: 12px; gap: 12px; }
.list { display: flex; flex-direction: column; gap: 4px; }
.list-row { display: flex; align-items: center; gap: 10px; padding: 8px 10px; border: 1px solid var(--border, #eee); border-radius: 8px; }
.list-row-main { flex: 1; min-width: 0; }
.list-row-title { font-weight: 600; }
.list-row-sub { font-size: 12px; color: var(--muted, #777); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
```

- [ ] **Step 5: Build + manual check**

Run: `cd ui && trunk build`
Expected: builds. Manual: Skills pane lists all skills with checkboxes; toggling a bundled skill persists (reopen shows the state); "Add SKILL.md"/"Add folder" install a skill that then appears with a remove button; removing a user skill drops it from the list.

- [ ] **Step 6: Commit**

```bash
git add ui/src/main.rs ui/src/i18n.rs ui/styles.css
git commit -m "feat(ui): skills section with enable/disable, add, remove"
```

---

### Task D3: Connections section (bio-tools toggle + connection list + add/edit form + test)

**Files:**
- Modify: `ui/src/main.rs`
- Modify: `ui/src/i18n.rs`
- Modify: `ui/styles.css`

**Interfaces:**
- Consumes: backend `list_mcp_connections` (returns `{ bio_tools_enabled, connections: [...] }`), `set_bio_tools_enabled`, `add_mcp_connection`, `update_mcp_connection`, `delete_mcp_connection`, `set_mcp_connection_enabled`, `test_mcp_connection`.
- Produces: signals `conns_view: RwSignal<Option<ConnView>>`, `conn_form: RwSignal<Option<ConnForm>>` (None = closed, Some = add/edit), `conn_test_msg: RwSignal<Option<(bool,String)>>`; the "connections" pane markup.

- [ ] **Step 1: Add UI-side types + signals**

```rust
#[derive(Clone, serde::Deserialize)]
struct ConnRow { id: String, name: String, enabled: bool, transport: ConnTransport }
#[derive(Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum ConnTransport {
    Stdio { command: String, #[serde(default)] args: Vec<String>, #[serde(default)] env: Vec<(String,String)>, #[serde(default)] cwd: Option<String> },
    Http  { url: String, #[serde(default)] headers: Vec<(String,String)> },
}
#[derive(Clone, serde::Deserialize)]
struct ConnView { bio_tools_enabled: bool, connections: Vec<ConnRow> }

// Simple flat form state (kind + raw text fields; args/env/headers entered as text, parsed on save).
#[derive(Clone, Default)]
struct ConnForm { id: Option<String>, name: String, kind: String, command: String, args: String, url: String, headers: String }
```

Signals: `let conns_view = create_rw_signal(None::<ConnView>); let conn_form = create_rw_signal(None::<ConnForm>); let conn_test_msg = create_rw_signal(None::<(bool,String)>);`

- [ ] **Step 2: Add a refresh action**

```rust
let refresh_conns = move || {
    spawn_local(async move {
        let v = invoke("list_mcp_connections", JsValue::UNDEFINED).await;
        if let Ok(view) = serde_wasm_bindgen::from_value::<ConnView>(v) { conns_view.set(Some(view)); }
    });
};
```

Call on modal open and on switching to the connections section.

- [ ] **Step 3: Render the connections pane**

Layout: a top row with the bio-tools toggle; the list of connections (name + kind badge + test/edit/delete + enable checkbox); an "Add connection" button that opens `conn_form`; and the form (rendered when `conn_form` is `Some`) with a kind selector (Local stdio / Remote URL) showing the relevant fields, plus Test / Save / Cancel.

```rust
{move || (settings_section.get() == "connections").then(|| view! {
    <div class="settings-pane">
        <span class="hint">{move || t(locale.get(), "settings.applies_new_session")}</span>
        // Bio-tools built-in toggle
        <div class="list-row">
            <div class="list-row-main">
                <div class="list-row-title">{move || t(locale.get(), "conn.biotools")}</div>
                <div class="list-row-sub">{move || t(locale.get(), "conn.biotools.desc")}</div>
            </div>
            <input type="checkbox"
                prop:checked=move || conns_view.get().map(|v| v.bio_tools_enabled).unwrap_or(true)
                on:change=move |ev| {
                    let on = event_target_checked(&ev);
                    spawn_local(async move {
                        let arg = to_value(&serde_json::json!({ "enabled": on })).unwrap();
                        let _ = invoke_checked("set_bio_tools_enabled", arg).await;
                        refresh_conns();
                    });
                } />
        </div>
        // User connections
        <div class="list">
            <For each=move || conns_view.get().map(|v| v.connections).unwrap_or_default() key=|c| c.id.clone() let:c>
                { /* row: title = c.name, badge = kind, buttons: edit (opens conn_form prefilled),
                     delete (delete_mcp_connection), checkbox (set_mcp_connection_enabled) */ }
            </For>
        </div>
        <div class="row">
            <button on:click=move |_| conn_form.set(Some(ConnForm { kind: "stdio".into(), ..Default::default() }))>
                {move || t(locale.get(), "conn.add")}</button>
        </div>
        // Add/edit form
        {move || conn_form.get().map(|f| view! {
            <div class="conn-form">
                <label>{move || t(locale.get(),"conn.name")}
                    <input prop:value=f.name.clone() on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.name = event_target_input(&ev).value(); }) /></label>
                <label>{move || t(locale.get(),"conn.kind")}
                    <select prop:value=f.kind.clone() on:change=move |ev| conn_form.update(|o| if let Some(o)=o { o.kind = event_target_value(&ev); })>
                        <option value="stdio">{move || t(locale.get(),"conn.kind.stdio")}</option>
                        <option value="http">{move || t(locale.get(),"conn.kind.http")}</option>
                    </select></label>
                // stdio fields
                {move || (conn_form.get().map(|f| f.kind).as_deref() == Some("stdio")).then(|| view!{
                    <label>{move || t(locale.get(),"conn.command")}
                        <input prop:value=conn_form.get().map(|f|f.command).unwrap_or_default() on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.command = event_target_input(&ev).value(); }) /></label>
                    <label>{move || t(locale.get(),"conn.args")}
                        <input placeholder="arg1 arg2" prop:value=conn_form.get().map(|f|f.args).unwrap_or_default() on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.args = event_target_input(&ev).value(); }) /></label>
                })}
                // http fields
                {move || (conn_form.get().map(|f| f.kind).as_deref() == Some("http")).then(|| view!{
                    <label>{move || t(locale.get(),"conn.url")}
                        <input placeholder="https://host/mcp" prop:value=conn_form.get().map(|f|f.url).unwrap_or_default() on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.url = event_target_input(&ev).value(); }) /></label>
                    <label>{move || t(locale.get(),"conn.headers")}
                        <input placeholder="Authorization: Bearer xxx" prop:value=conn_form.get().map(|f|f.headers).unwrap_or_default() on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.headers = event_target_input(&ev).value(); }) /></label>
                })}
                {move || conn_test_msg.get().map(|(ok,msg)| view!{ <div class="settings-status" class:ok=ok class:fail=move||!ok>{msg}</div> })}
                <div class="row">
                    <button on:click=move |_| { let f = conn_form.get().unwrap_or_default();
                        spawn_local(async move {
                            let conn = build_conn_json(&f, false);
                            match invoke_checked("test_mcp_connection", to_value(&serde_json::json!({"conn": conn})).unwrap()).await {
                                Ok(v) => { let n = v.as_f64().unwrap_or(0.0) as i64; conn_test_msg.set(Some((true, format!("OK — {n} tools")))); }
                                Err(e) => conn_test_msg.set(Some((false, format!("{e:?}")))),
                            }
                        });
                    }>{move || t(locale.get(),"conn.test")}</button>
                    <button on:click=move |_| { conn_form.set(None); conn_test_msg.set(None); }>{move || t(locale.get(),"settings.cancel")}</button>
                    <button class="primary" on:click=move |_| { let f = conn_form.get().unwrap_or_default();
                        spawn_local(async move {
                            let editing = f.id.is_some();
                            let conn = build_conn_json(&f, true);
                            let cmd = if editing { "update_mcp_connection" } else { "add_mcp_connection" };
                            if invoke_checked(cmd, to_value(&serde_json::json!({"conn": conn})).unwrap()).await.is_ok() {
                                conn_form.set(None); conn_test_msg.set(None); refresh_conns();
                            }
                        });
                    }>{move || t(locale.get(),"settings.save")}</button>
                </div>
            </div>
        })}
    </div>
}.into_view())}
```

- [ ] **Step 4: Add the `build_conn_json` helper**

Parses the flat form into the backend `McpConnection` JSON shape. `id` is a client-generated uuid on add (use a simple time-free random via `js_sys` or a counter — but `Math.random`-based is fine for an id; here use `web_sys`/`js_sys::Date` is unavailable in some contexts, so derive from `name` + a monotonic counter signal, or use `uuid` if already a UI dep). Simplest: generate in Rust with a `nanoid`-free scheme — concatenate name with a random suffix from `js_sys::Math::random()`.

```rust
fn build_conn_json(f: &ConnForm, assign_id: bool) -> serde_json::Value {
    let id = f.id.clone().unwrap_or_else(|| if assign_id {
        format!("conn-{}", (js_sys::Math::random() * 1e9) as u64)
    } else { "test".into() });
    let transport = if f.kind == "http" {
        let headers: Vec<(String,String)> = f.headers.lines().filter_map(|l| l.split_once(':').map(|(k,v)| (k.trim().to_string(), v.trim().to_string()))).collect();
        serde_json::json!({ "kind": "http", "url": f.url.trim(), "headers": headers })
    } else {
        let args: Vec<String> = f.args.split_whitespace().map(|s| s.to_string()).collect();
        serde_json::json!({ "kind": "stdio", "command": f.command.trim(), "args": args, "env": [], "cwd": null })
    };
    serde_json::json!({ "id": id, "name": f.name.trim(), "enabled": true, "transport": transport })
}
```

> The edit button prefills `conn_form` from a `ConnRow` (map `ConnTransport::Stdio` → kind "stdio", join args with spaces; `Http` → kind "http", join headers as `k: v` lines), keeping `id: Some(row.id)` so Save routes to `update_mcp_connection`.

- [ ] **Step 5: i18n + CSS**

i18n keys (en/zh):

```
conn.biotools       → "Bio-tools (built-in)" / "Bio-tools（内置）"
conn.biotools.desc  → "Bundled bioinformatics MCP servers." / "内置的生物信息学 MCP 服务。"
conn.add            → "Add connection" / "添加连接"
conn.name           → "Name" / "名称"
conn.kind           → "Type" / "类型"
conn.kind.stdio     → "Local command" / "本地命令"
conn.kind.http      → "Remote URL" / "远程 URL"
conn.command        → "Command" / "命令"
conn.args           → "Arguments" / "参数"
conn.url            → "URL" / "URL"
conn.headers        → "Headers (one per line)" / "请求头（每行一个）"
conn.test           → "Test" / "测试"
```

CSS:

```css
.conn-form { margin-top: 12px; padding: 12px; border: 1px solid var(--border,#eee); border-radius: 8px; display: flex; flex-direction: column; gap: 8px; }
.badge { font-size: 11px; padding: 2px 6px; border-radius: 999px; background: var(--surface-2,#eef1f0); color: var(--muted,#555); }
```

- [ ] **Step 6: Build + manual check**

Run: `cd ui && trunk build`
Expected: builds. Manual: Connections pane shows the Bio-tools toggle (persists); "Add connection" opens the form; choosing Local command vs Remote URL swaps the fields; Test on a valid local stdio server (e.g. an installed MCP) returns "OK — N tools"; Save adds a row; edit/delete/enable-toggle work and persist across reopen.

- [ ] **Step 7: Commit**

```bash
git add ui/src/main.rs ui/src/i18n.rs ui/styles.css
git commit -m "feat(ui): connections section — bio-tools toggle + local/remote MCP CRUD + test"
```

---

## Self-Review

**Spec coverage:**
- Multi-section Settings page (General/Skills/Connections) → D1/D2/D3. ✓
- Skill enable/disable + persistence (`disabled_skills`) → B1/B2/B3. ✓
- Add skill via file picker (SKILL.md + folder) → B3 (`pick_skill_source`/`install_skill`) + D2. ✓
- Remove user skill (bundled = disable only) → B3 (`remove_skill`) + D2. ✓
- `mcp_connections` model + persistence → C1. ✓
- Local stdio + remote HTTP transport → A1/A2 + C2 (`connect_mcp`). ✓
- Bio-tools toggle (`bio_tools_enabled`) → C1/C2 + D3. ✓
- Connection CRUD + enable + test → C2 + D3. ✓
- Wiring gated by settings, applied to new sessions / idle-agent reset → B2/C2 (`reset_idle_agents`). ✓
- Public wisp-mcp API unchanged (`McpTool` untouched) → A1 keeps signatures; `launch_with_command` is additive. ✓
- Tests: SkillIndex filter (B1), connection serde roundtrip (C1), SSE parse (A2), disabled_skills parse (B2). ✓

**Deferred (per spec, intentional):** `.zip` upload, GitHub import, splitting bio-tools into 87 connections, per-project disabled skills, mid-conversation hot-swap.

**Type consistency check:**
- `SkillInfo { name, description, enabled, builtin, dir }` used in B3 and consumed as `SkillRow` in D2 (same field names). ✓
- `McpConnection`/`McpTransport` (C1) ↔ `ConnRow`/`ConnTransport` (D3) — same serde tag `kind`, same field names. ✓ Backend `env`/`headers` are `Vec<(String,String)>`; the UI form emits `headers` as array-of-pairs and `env` as `[]` — matches serde. ✓
- `wire_python_and_mcp` becomes 3-arg in C2; the B2 call site note explicitly defers the arg change to C2 to avoid a half-applied signature. ✓
- `reset_idle_agents` defined once (B3) and reused by `set_skill_enabled`, connection commands, and `set_settings` (refactor). ✓
- `connect_mcp` uses `McpClient::launch_with_command` (added in C2) + `connect_http` (A2). ✓

**Ordering note:** A → B → C → D. B2's agent-build edit and C2's `wire_python_and_mcp` signature change touch adjacent lines in `send_message`; C2 must update the call site introduced in B2. Flagged in both tasks.
