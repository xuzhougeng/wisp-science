# Specialists (专家系统) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** User-definable agent personas (instructions + skill/MCP subset + directly-bound model), selectable per session; the existing one-shot review becomes the built-in Reviewer specialist; specialists can be created by form or by chat via a `save_specialist` tool.

**Architecture:** A new `specialists.rs` module stores specialists as a JSON array in the sqlite settings (same pattern as `model_profiles`), with the builtin Reviewer materialized on first read. Personas take effect at the existing Agent-construction point in `send_message` (prompt append, skill/connector whitelist filters, model override) — no `agent_loop` changes. Spec: `docs/superpowers/specs/2026-07-09-specialists-design.md`.

**Tech Stack:** Rust (tauri backend, `cargo test -p wisp-tauri`), Leptos 0.6 CSR UI (`cd ui && cargo test` + `cargo check --target wasm32-unknown-unknown`), Playwright (`cd ui-tests && npx playwright test`).

## Global Constraints

- No intermediate model-slot layer: specialists bind `model_id` directly; `""` = follow the active model; dangling id falls back to the active model.
- Builtin Reviewer (`id: "reviewer"`): not deletable; `instructions` read-only; other fields editable and persisted like any row.
- `VISION_KEY` / vision assignment is untouched.
- All new backend strings user-visible in the UI need `en` + `zh` entries in `ui/src/i18n.rs`.
- Every commit message ends with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.
- Branch: `feat/specialists` (already created; spec committed on it).

## Delivery boundaries

- PR 1 = Tasks 1–5 (backend), PR 2 = Tasks 6–8 (UI), PR 3 = Tasks 9–10 (chat creation). Each PR ends with the full test sweep: `cargo test --workspace`, `cd ui && cargo test && cargo check --target wasm32-unknown-unknown`, `cd ui-tests && npx playwright test`.

---

### Task 1: `specialists.rs` — data model, storage, builtin Reviewer, CRUD

**Files:**
- Create: `src-tauri/src/specialists.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod specialists;` next to `mod models;` around line 19)
- Modify: `src-tauri/src/models.rs` (add `profile_llm`, reuse by Task 2/4)

**Interfaces:**
- Produces: `Specialist` struct; `pub async fn ensure(store: &Store) -> Vec<Specialist>`; `pub async fn get(store: &Store, id: &str) -> Option<Specialist>`; `pub async fn upsert(store: &Store, spec: Specialist) -> Result<Vec<Specialist>, String>`; `pub async fn remove(store: &Store, id: &str) -> Result<Vec<Specialist>, String>`; tauri commands `list_specialists`, `save_specialist_cmd`, `remove_specialist`; `models::profile_llm(store, id) -> Option<(String, String, String, String, u64, String)>` (provider, api_url, model, api_key, max_tokens, reasoning_effort).

- [ ] **Step 1: Write the failing tests** (bottom of the new `src-tauri/src/specialists.rs`; the module skeleton in Step 3 makes them compile)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    async fn test_store() -> (wisp_store::Store, std::path::PathBuf) {
        let tmp = std::env::temp_dir().join(format!("wisp_spec_{}.sqlite", uuid::Uuid::new_v4()));
        (wisp_store::Store::open(&tmp).await.unwrap(), tmp)
    }

    #[tokio::test]
    async fn ensure_materializes_builtin_reviewer_once() {
        let (store, tmp) = test_store().await;
        let list = ensure(&store).await;
        assert_eq!(list.len(), 1);
        let r = &list[0];
        assert_eq!(r.id, "reviewer");
        assert!(r.builtin);
        assert_eq!(r.instructions, crate::review::REVIEWER_RUBRIC);
        // Second read does not duplicate it.
        assert_eq!(ensure(&store).await.len(), 1);
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn upsert_roundtrip_and_fresh_id() {
        let (store, tmp) = test_store().await;
        let spec = Specialist {
            id: String::new(),
            name: "Paper hunter".into(),
            icon: "search".into(),
            color: "clay".into(),
            description: "finds papers".into(),
            instructions: "You hunt papers.".into(),
            model_id: "m1".into(),
            skills: Some(vec!["bear-support".into()]),
            connectors: None,
            builtin: false,
        };
        let list = upsert(&store, spec).await.unwrap();
        let created = list.iter().find(|s| !s.builtin).unwrap();
        assert_eq!(created.id, "sp1");
        assert_eq!(created.skills.as_deref(), Some(&["bear-support".to_string()][..]));
        // Edit by id keeps the id.
        let mut edited = created.clone();
        edited.name = "Paper hunter 2".into();
        let list = upsert(&store, edited).await.unwrap();
        assert_eq!(list.iter().filter(|s| !s.builtin).count(), 1);
        assert_eq!(list.iter().find(|s| s.id == "sp1").unwrap().name, "Paper hunter 2");
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn builtin_reviewer_guards() {
        let (store, tmp) = test_store().await;
        ensure(&store).await;
        assert!(remove(&store, "reviewer").await.is_err());
        // Editing the builtin keeps instructions but accepts a model change.
        let mut r = get(&store, "reviewer").await.unwrap();
        r.instructions = "haha".into();
        r.model_id = "m2".into();
        let list = upsert(&store, r).await.unwrap();
        let r = list.iter().find(|s| s.id == "reviewer").unwrap();
        assert_eq!(r.instructions, crate::review::REVIEWER_RUBRIC);
        assert_eq!(r.model_id, "m2");
        let _ = std::fs::remove_file(&tmp);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p wisp-tauri specialists 2>&1 | tail -5`
Expected: compile error (module missing) — that is the failure for this step.

- [ ] **Step 3: Write the module**

```rust
//! Specialists (专家): user-definable agent personas — instructions plus a
//! skill/MCP subset and a directly-bound model, selectable per session.
//! Stored as a JSON array under the `specialists` settings key (same pattern
//! as `model_profiles`). The builtin Reviewer is materialized into the list on
//! first read so user edits to its model binding persist like any other row.

use serde::{Deserialize, Serialize};
use tauri::State;
use wisp_store::Store;

pub const SPECIALISTS_KEY: &str = "specialists";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Specialist {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub instructions: String,
    /// "" = follow the active model; dangling ids fall back to active too.
    #[serde(default)]
    pub model_id: String,
    /// None = inherit the project skill config; Some = whitelist of skill names.
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    /// None = inherit; Some = whitelist of connector slugs / MCP connection ids.
    #[serde(default)]
    pub connectors: Option<Vec<String>>,
    #[serde(default)]
    pub builtin: bool,
}

fn builtin_reviewer() -> Specialist {
    Specialist {
        id: "reviewer".into(),
        name: "Reviewer".into(),
        icon: "review".into(),
        color: "clay".into(),
        description: "Traces a session transcript and reports fabrication, hallucination, or plan deviation.".into(),
        instructions: crate::review::REVIEWER_RUBRIC.into(),
        model_id: String::new(),
        skills: Some(vec![]), // reviewer runs one-shot; skills are irrelevant
        connectors: Some(vec![]),
        builtin: true,
    }
}

async fn load_raw(store: &Store) -> Vec<Specialist> {
    store
        .get_setting(SPECIALISTS_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<Vec<Specialist>>(&s).ok())
        .unwrap_or_default()
}

async fn save_raw(store: &Store, list: &[Specialist]) -> Result<(), String> {
    let json = serde_json::to_string(list).map_err(|e| e.to_string())?;
    store
        .set_setting(SPECIALISTS_KEY, &json)
        .await
        .map_err(|e| e.to_string())
}

/// Load the list, materializing the builtin Reviewer if absent. The builtin's
/// `instructions` are always re-pinned to the compiled rubric so rubric
/// improvements ship without a migration.
pub async fn ensure(store: &Store) -> Vec<Specialist> {
    let mut list = load_raw(store).await;
    match list.iter_mut().find(|s| s.id == "reviewer") {
        Some(r) => {
            r.builtin = true;
            r.instructions = crate::review::REVIEWER_RUBRIC.into();
        }
        None => list.insert(0, builtin_reviewer()),
    }
    list
}

pub async fn get(store: &Store, id: &str) -> Option<Specialist> {
    ensure(store).await.into_iter().find(|s| s.id == id)
}

fn fresh_id(existing: &[Specialist]) -> String {
    for n in 1..10_000 {
        let id = format!("sp{n}");
        if !existing.iter().any(|s| s.id == id) {
            return id;
        }
    }
    "sp".into()
}

/// Create (empty id) or update (existing id). Builtin rows keep their
/// compiled instructions and can never lose `builtin`.
pub async fn upsert(store: &Store, mut spec: Specialist) -> Result<Vec<Specialist>, String> {
    if spec.name.trim().is_empty() {
        return Err("Specialist name is required.".into());
    }
    let mut list = ensure(store).await;
    if spec.id.trim().is_empty() {
        spec.id = fresh_id(&list);
    }
    if let Some(existing) = list.iter_mut().find(|s| s.id == spec.id) {
        if existing.builtin {
            spec.builtin = true;
            spec.instructions = existing.instructions.clone();
        }
        *existing = spec;
    } else {
        spec.builtin = false;
        list.push(spec);
    }
    save_raw(store, &list).await?;
    Ok(ensure(store).await)
}

pub async fn remove(store: &Store, id: &str) -> Result<Vec<Specialist>, String> {
    let mut list = ensure(store).await;
    if list.iter().any(|s| s.id == id && s.builtin) {
        return Err("Built-in specialists cannot be removed.".into());
    }
    list.retain(|s| s.id != id);
    save_raw(store, &list).await?;
    Ok(ensure(store).await)
}

#[tauri::command]
pub async fn list_specialists(state: State<'_, crate::AppState>) -> Result<Vec<Specialist>, String> {
    Ok(ensure(&state.store).await)
}

#[tauri::command]
pub async fn save_specialist_cmd(
    state: State<'_, crate::AppState>,
    spec: Specialist,
) -> Result<Vec<Specialist>, String> {
    upsert(&state.store, spec).await
}

#[tauri::command]
pub async fn remove_specialist(
    state: State<'_, crate::AppState>,
    id: String,
) -> Result<Vec<Specialist>, String> {
    remove(&state.store, &id).await
}
```

Also add to `src-tauri/src/lib.rs` module list (next to `mod models;`):

```rust
mod specialists;
```

And in `src-tauri/src/models.rs`, below `active_llm_advanced`:

```rust
/// Full LLM config for one profile id: (provider, api_url, model, api_key,
/// max_tokens, reasoning_effort). None when the id doesn't exist.
pub async fn profile_llm(
    store: &wisp_store::Store,
    id: &str,
) -> Option<(String, String, String, String, u64, String)> {
    let profiles = ensure(store).await;
    let p = profiles.iter().find(|p| p.id == id)?;
    Some((
        p.provider.clone(),
        p.api_url.clone(),
        p.model.clone(),
        key_for(&p.id),
        p.max_tokens,
        p.reasoning_effort.clone(),
    ))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p wisp-tauri specialists 2>&1 | tail -5`
Expected: `3 passed`

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/specialists.rs src-tauri/src/lib.rs src-tauri/src/models.rs
git commit -m "feat(specialists): data model, storage, builtin Reviewer, CRUD

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: model resolution + per-session selection commands

**Files:**
- Modify: `src-tauri/src/specialists.rs`
- Test: same file's `tests` module

**Interfaces:**
- Consumes: `models::profile_llm`, `crate::load_settings`, `models::active_llm_advanced`.
- Produces: `pub async fn specialist_llm(store: &Store, spec: &Specialist) -> (String, String, String, String, u64, String)`; setting key `frame_specialist:<frame_id>`; `pub async fn session_specialist(store: &Store, frame_id: &str) -> Option<Specialist>`; tauri commands `set_session_specialist`, `get_session_specialist`.

- [ ] **Step 1: Write the failing tests** (append to `specialists.rs` tests)

```rust
    #[tokio::test]
    async fn specialist_llm_falls_back_to_active_for_empty_or_dangling() {
        let (store, tmp) = test_store().await;
        // No model profiles configured: active resolution still returns the
        // env/default fallback chain from load_settings.
        let spec = Specialist { model_id: "no-such".into(), ..builtin_reviewer() };
        let (provider, api_url, model, _key, _mt, _re) = specialist_llm(&store, &spec).await;
        assert!(!provider.is_empty());
        assert!(!api_url.is_empty());
        assert!(!model.is_empty());
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn session_specialist_set_get_and_lock() {
        let (store, tmp) = test_store().await;
        ensure(&store).await;
        store.create_project("p1", "proj", "").await.unwrap();
        store.create_frame("f1", "p1", "OPERON", "m").await.unwrap();
        set_frame_specialist(&store, "f1", "reviewer").await.unwrap();
        assert_eq!(session_specialist(&store, "f1").await.unwrap().id, "reviewer");
        // Clearing works.
        set_frame_specialist(&store, "f1", "").await.unwrap();
        assert!(session_specialist(&store, "f1").await.is_none());
        let _ = std::fs::remove_file(&tmp);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p wisp-tauri specialists 2>&1 | tail -5`
Expected: compile error — `specialist_llm` / `set_frame_specialist` not found.

- [ ] **Step 3: Implement** (append to `specialists.rs`)

```rust
/// LLM config for a specialist: its bound profile, or the active-model chain
/// when unbound/dangling (soft fallback — personas are not hard capabilities).
pub async fn specialist_llm(
    store: &Store,
    spec: &Specialist,
) -> (String, String, String, String, u64, String) {
    if !spec.model_id.trim().is_empty() {
        if let Some(cfg) = crate::models::profile_llm(store, &spec.model_id).await {
            return cfg;
        }
    }
    let (provider, api_url, model, api_key) = crate::load_settings(store).await;
    let (max_tokens, reasoning_effort) = crate::models::active_llm_advanced(store).await;
    (provider, api_url, model, api_key, max_tokens, reasoning_effort)
}

fn frame_key(frame_id: &str) -> String {
    format!("frame_specialist:{frame_id}")
}

pub async fn set_frame_specialist(store: &Store, frame_id: &str, id: &str) -> Result<(), String> {
    store
        .set_setting(&frame_key(frame_id), id)
        .await
        .map_err(|e| e.to_string())
}

pub async fn session_specialist(store: &Store, frame_id: &str) -> Option<Specialist> {
    let id = store.get_setting(&frame_key(frame_id)).await.ok().flatten()?;
    if id.trim().is_empty() {
        return None;
    }
    get(store, &id).await
}

/// The UI disables the picker once a session has messages; this backend guard
/// enforces the same rule for any other caller.
#[tauri::command]
pub async fn set_session_specialist(
    state: State<'_, crate::AppState>,
    frame_id: String,
    id: String,
) -> Result<(), String> {
    let msgs = state
        .store
        .load_messages(&frame_id)
        .await
        .map_err(|e| format!("{e}"))?;
    if msgs.iter().any(|m| m.role != wisp_llm::Role::System) {
        return Err("Specialist is locked once the session has messages.".into());
    }
    if !id.is_empty() && get(&state.store, &id).await.is_none() {
        return Err(format!("Unknown specialist '{id}'."));
    }
    set_frame_specialist(&state.store, &frame_id, &id).await
}

#[tauri::command]
pub async fn get_session_specialist(
    state: State<'_, crate::AppState>,
    frame_id: String,
) -> Result<Option<Specialist>, String> {
    Ok(session_specialist(&state.store, &frame_id).await)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p wisp-tauri specialists 2>&1 | tail -5`
Expected: `5 passed`

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/specialists.rs
git commit -m "feat(specialists): model resolution and per-session selection

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: persona takes effect at Agent construction

**Files:**
- Modify: `src-tauri/src/lib.rs` — `send_message` (config block ~line 1602 and agent-build block ~line 1655) and `wire_python_and_mcp` (~line 1420)
- Test: `src-tauri/src/lib.rs` tests module (helper-level tests)

**Interfaces:**
- Consumes: `specialists::session_specialist`, `specialists::specialist_llm`, `SkillIndex::filtered_by_names(Option<&HashSet<String>>)`.
- Produces: `fn specialist_prompt_section(spec: &Specialist) -> String`; `wire_python_and_mcp(agent, app_data, store, connector_allow: Option<&HashSet<String>>)` (existing two call sites updated: `send_message` passes the specialist's set, any other caller passes `None`).

- [ ] **Step 1: Write the failing tests** (in the existing `#[cfg(test)] mod tests` at the bottom of `lib.rs`)

```rust
    #[test]
    fn specialist_prompt_section_appends_identity() {
        let spec = crate::specialists::Specialist {
            id: "sp1".into(),
            name: "Paper hunter".into(),
            icon: String::new(),
            color: String::new(),
            description: "ignored".into(),
            instructions: "You hunt papers.".into(),
            model_id: String::new(),
            skills: None,
            connectors: None,
            builtin: false,
        };
        let s = crate::specialist_prompt_section(&spec);
        assert!(s.starts_with("\n\n## Specialist: Paper hunter\n"));
        assert!(s.contains("You hunt papers."));
        assert!(!s.contains("ignored"), "description must not enter the prompt");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p wisp-tauri specialist_prompt 2>&1 | tail -3`
Expected: compile error — function not defined.

- [ ] **Step 3: Implement**

Add near `active_skill_index` in `lib.rs`:

```rust
/// Identity section appended after the base system prompt when a session has
/// a specialist. Description is UI-only and deliberately excluded.
fn specialist_prompt_section(spec: &specialists::Specialist) -> String {
    format!("\n\n## Specialist: {}\n{}", spec.name, spec.instructions)
}
```

Change `wire_python_and_mcp`'s signature and the two filter points:

```rust
async fn wire_python_and_mcp(
    agent: &mut wisp_core::Agent,
    app_data: &std::path::Path,
    store: &Store,
    connector_allow: Option<&HashSet<String>>,
) -> Vec<String> {
```

Inside, where bio-domain `skip`/`all_off` are computed from `disabled`, extend both with the whitelist (a domain not in the allow set is treated as disabled):

```rust
        let disabled = load_disabled_connectors(store).await;
        let domains = bio_domains();
        let blocked = |slug: &str| {
            disabled.contains(slug)
                || connector_allow.is_some_and(|allow| !allow.contains(slug))
        };
        let all_off = !domains.is_empty() && domains.iter().all(|d| blocked(&d.slug));
        let skip: HashSet<String> = domains
            .iter()
            .filter(|d| blocked(&d.slug))
            .flat_map(|d| d.tools.iter().cloned())
            .collect();
```

And where user MCP connections are iterated (`load_mcp_connections(...)` + `c.enabled` filter), add:

```rust
        .filter(|c| connector_allow.is_none_or(|allow| allow.contains(&c.id)))
```

In `send_message`: after `let frame_id = ...` is resolved, load the persona, then use it for config and agent construction:

```rust
    let specialist = specialists::session_specialist(&state.store, &frame_id).await;
```

Replace the config block (`load_settings` + `active_llm_advanced` + `build_provider_config`) usage so a persona overrides the model — note the config block currently runs before `frame_id` exists, so move it after `frame_id` resolution (it has no other dependents in between):

```rust
    let (provider, api_url, model, api_key, max_tokens, reasoning_effort) = match &specialist {
        Some(spec) => specialists::specialist_llm(&state.store, spec).await,
        None => {
            let (p, u, m, k) = load_settings(&state.store).await;
            let (mt, re) = models::active_llm_advanced(&state.store).await;
            (p, u, m, k, mt, re)
        }
    };
    let cfg = build_provider_config(&provider, &api_url, &api_key, &model, max_tokens, &reasoning_effort)?;
```

In the agent-build block (`if guard.is_none()`), apply skills filter + prompt append, and pass the connector whitelist:

```rust
        let skills = active_skill_index(&state.store, &ap).await;
        let skills = match specialist.as_ref().and_then(|s| s.skills.as_ref()) {
            Some(names) => {
                let set: HashSet<String> = names.iter().cloned().collect();
                Arc::new(skills.filtered_by_names(Some(&set)))
            }
            None => skills,
        };
```

After the existing `agent.seed_system_prompt(&skills, compute);` line:

```rust
        if let Some(spec) = &specialist {
            if agent.ctx.messages.len() == 1 && !spec.instructions.trim().is_empty() {
                let section = specialist_prompt_section(spec);
                if let Some(m) = agent.ctx.messages.first_mut() {
                    if let wisp_llm::Content::Text(t) = &mut m.content {
                        t.push_str(&section);
                    }
                }
            }
        }
```

And the wire call becomes:

```rust
        let connector_allow: Option<HashSet<String>> = specialist
            .as_ref()
            .and_then(|s| s.connectors.as_ref())
            .map(|v| v.iter().cloned().collect());
        let wire_errors =
            wire_python_and_mcp(&mut agent, &state.app_data, &state.store, connector_allow.as_ref()).await;
```

Check for any other `wire_python_and_mcp(` caller (`grep -n 'wire_python_and_mcp(' src-tauri/src/lib.rs`) and pass `None` there.

- [ ] **Step 4: Run tests + full crate check**

Run: `cargo test -p wisp-tauri 2>&1 | grep -E 'test result|FAILED' | head -4`
Expected: all pass (new test included), no failures.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(specialists): persona takes effect at agent construction

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: `review_session` sources config from the Reviewer specialist

**Files:**
- Modify: `src-tauri/src/lib.rs:1856-1927` (`review_session`)
- Test: `src-tauri/src/specialists.rs` tests

**Interfaces:**
- Consumes: `specialists::get`, `specialists::specialist_llm`.
- Produces: unchanged command surface; review instructions/model now come from the `reviewer` specialist row.

- [ ] **Step 1: Write the failing test** (append to `specialists.rs` tests)

```rust
    #[tokio::test]
    async fn reviewer_model_binding_feeds_review_config() {
        let (store, tmp) = test_store().await;
        let mut r = get(&store, "reviewer").await.unwrap();
        r.model_id = "does-not-exist".into();
        upsert(&store, r).await.unwrap();
        // Dangling binding falls back to the active chain — never errors.
        let spec = get(&store, "reviewer").await.unwrap();
        let (_p, _u, model, _k, _mt, _re) = specialist_llm(&store, &spec).await;
        assert!(!model.is_empty());
        let _ = std::fs::remove_file(&tmp);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p wisp-tauri reviewer_model_binding 2>&1 | tail -3`
Expected: FAIL only if Tasks 1–2 are incomplete; if it passes immediately, continue (the test pins behavior for the refactor below).

- [ ] **Step 3: Re-source `review_session`**

Replace the config lines inside `review_session` (currently `load_settings` + `active_llm_advanced` + `Message::system(review::REVIEWER_RUBRIC)`):

```rust
        let reviewer = specialists::get(&state.store, "reviewer")
            .await
            .ok_or_else(|| "Reviewer specialist missing.".to_string())?;
        let (provider, api_url, model, api_key, max_tokens, reasoning_effort) =
            specialists::specialist_llm(&state.store, &reviewer).await;
        let cfg = build_provider_config(
            &provider,
            &api_url,
            &api_key,
            &model,
            max_tokens,
            &reasoning_effort,
        )?;
        let llm = wisp_llm::build(cfg);

        let review_msgs = vec![
            Message::system(reviewer.instructions.clone()),
            Message::user(transcript),
        ];
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p wisp-tauri 2>&1 | grep -E 'test result|FAILED' | head -4`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/specialists.rs
git commit -m "feat(specialists): review_session runs the builtin Reviewer specialist

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: register commands; PR 1 sweep

**Files:**
- Modify: `src-tauri/src/lib.rs` invoke handler list (~line 4850, next to `review_session`)

- [ ] **Step 1: Register commands**

Add to the `tauri::generate_handler![...]` list:

```rust
            specialists::list_specialists,
            specialists::save_specialist_cmd,
            specialists::remove_specialist,
            specialists::set_session_specialist,
            specialists::get_session_specialist,
```

- [ ] **Step 2: Full backend sweep**

Run: `cargo test --workspace 2>&1 | grep -E 'test result|FAILED' | tail -8` and `cargo check -p wisp-tauri`
Expected: all green.

- [ ] **Step 3: Commit and open PR 1**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(specialists): register tauri commands

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
git push -u origin feat/specialists
gh pr create --base main --title "feat: specialists backend (专家系统 PR 1/3)" --body "Backend for docs/superpowers/specs/2026-07-09-specialists-design.md: data model + storage + builtin Reviewer, per-session selection, persona hooks at agent construction, review_session re-source. UI lands in PR 2, chat creation in PR 3.

🤖 Generated with [Claude Code](https://claude.com/claude-code)"
```

---

### Task 6: UI — dto, args, Specialists settings page

**Files:**
- Modify: `ui/src/dto.rs` (add `Specialist` mirror struct next to `ModelProfile`)
- Modify: `ui/src/main.rs` (settings nav + section, form state, save/remove handlers; nav anchor `settings_section_label` ~line 874; section blocks after "models" ~line 5644)
- Modify: `ui/src/i18n.rs` (new keys, en + zh)

**Interfaces:**
- Consumes: tauri commands from Task 5.
- Produces: `dto::Specialist` (serde mirror, `PartialEq + Clone`); signals `specialists: RwSignal<Vec<Specialist>>`, `specialist_form: RwSignal<Option<Specialist>>`; settings section id `"specialists"`.

- [ ] **Step 1: dto mirror** (in `ui/src/dto.rs`, after `ModelProfile`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct Specialist {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)] pub(crate) icon: String,
    #[serde(default)] pub(crate) color: String,
    #[serde(default)] pub(crate) description: String,
    #[serde(default)] pub(crate) instructions: String,
    #[serde(default)] pub(crate) model_id: String,
    #[serde(default)] pub(crate) skills: Option<Vec<String>>,
    #[serde(default)] pub(crate) connectors: Option<Vec<String>>,
    #[serde(default)] pub(crate) builtin: bool,
}
```

- [ ] **Step 2: i18n keys** (`ui/src/i18n.rs`, both `Locale::En` and `Locale::Zh` match arms; zh values in parentheses)

```
"settings.nav.specialists" => "Specialists" (专家)
"specialists.builtin" => "Built-in" (内置)
"specialists.custom" => "Custom" (自定义)
"specialists.add" => "Add specialist" (新建专家)
"specialists.add.scratch" => "Write from scratch" (从零开始)
"specialists.add.chat" => "Chat with Claude" (通过对话创建)
"specialists.name" => "Name" (名称)
"specialists.description" => "Description" (描述)
"specialists.instructions" => "Instructions" (指令)
"specialists.instructions.hint" => "Appended to the base prompt. Optional." (追加到基础提示词之后,可留空)
"specialists.model" => "Model" (模型)
"specialists.model.follow" => "Follow active model" (跟随当前模型)
"specialists.skills" => "Skills" (技能)
"specialists.inherit" => "Inherit project settings" (继承项目设置)
"specialists.remove" => "Remove" (删除)
"specialists.builtin_locked" => "Built-in instructions are read-only." (内置指令不可编辑)
```

- [ ] **Step 3: nav entry + section + form**

In `settings_section_label` add `"specialists" => t(loc, "settings.nav.specialists"),`. Add a nav button next to the models one (same `class:active` pattern, `go_settings_section("specialists")`).

App-level signals (near `let models = ...`):

```rust
    let specialists = create_rw_signal::<Vec<Specialist>>(vec![]);
    let specialist_form = create_rw_signal::<Option<Specialist>>(None);
```

Refresh helper (near `refresh_sessions`):

```rust
fn refresh_specialists(into: RwSignal<Vec<Specialist>>) {
    spawn_local(async move {
        let v = invoke("list_specialists", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<Specialist>>(v) {
            into.set(list);
        }
    });
}
```

Call `refresh_specialists(specialists);` where settings opens (`show_settings.set(true)` handler / `go_settings_section`).

Section block (after the `"models"` section, same structure — list rows grouped by `builtin`, click row → `specialist_form.set(Some(s.clone()))`; "Add specialist" → `specialist_form.set(Some(Specialist { id: String::new(), name: String::new(), icon: "review".into(), color: "clay".into(), description: String::new(), instructions: String::new(), model_id: String::new(), skills: None, connectors: None, builtin: false }))`). Form fields: name (text input), description (textarea), instructions (textarea, `prop:disabled` when `builtin` + hint `specialists.builtin_locked`), model (select: option "" = `specialists.model.follow`, options from `models.get()` by label/id), skills (checkbox "inherit" toggling `skills: None` vs whitelist list of checkboxes from the skills list already loaded for the Skills page). Save handler:

```rust
    let save_specialist_form = move |_| {
        let Some(spec) = specialist_form.get() else { return; };
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "spec": spec })).unwrap();
            match invoke_checked("save_specialist_cmd", arg).await {
                Ok(v) => {
                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<Specialist>>(v) {
                        specialists.set(list);
                    }
                    specialist_form.set(None);
                }
                Err(err) => {
                    // Same surface the model form uses for its failures.
                    model_form_msg.set(Some((false, localize_backend(locale.get_untracked(), &js_error_text(err)))));
                }
            }
        });
    };
```

Remove handler mirrors `remove_model` (`invoke_checked("remove_specialist", {id})`), only rendered when `!builtin`.

- [ ] **Step 4: Compile + host tests**

Run: `cd ui && cargo test 2>&1 | tail -3 && cargo check --target wasm32-unknown-unknown 2>&1 | tail -1`
Expected: pass / clean.

- [ ] **Step 5: Commit**

```bash
git add ui/src/dto.rs ui/src/i18n.rs ui/src/main.rs
git commit -m "feat(ui): specialists settings page

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 7: UI — per-session Specialist picker

**Files:**
- Modify: `ui/src/main.rs` (composer session-options popover; session header)
- Modify: `ui/src/i18n.rs`

**Interfaces:**
- Consumes: `set_session_specialist` / `get_session_specialist` commands; `specialists` signal.
- Produces: signal `session_specialist: RwSignal<Option<Specialist>>` refreshed on session switch.

- [ ] **Step 1: i18n keys**

```
"composer.specialist" => "Specialist" (专家)
"composer.specialist.none" => "None" (无)
"composer.specialist.locked" => "Locked after the first message" (首条消息后锁定)
```

- [ ] **Step 2: state + wiring**

Signal + refresh on active-session change (`create_effect` watching `active_session`):

```rust
    let session_specialist = create_rw_signal::<Option<Specialist>>(None);
    create_effect(move |_| {
        let Some(sid) = active_session.get() else { session_specialist.set(None); return; };
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "frameId": sid })).unwrap();
            let v = invoke("get_session_specialist", arg).await;
            session_specialist.set(serde_wasm_bindgen::from_value::<Option<Specialist>>(v).ok().flatten());
        });
    });
```

Picker submenu inside the existing session-options popover (pattern: the Compute submenu there): entries None + each of `specialists.get()`; disabled (`class:disabled` + no-op) when `items.with(|l| !l.is_empty())` — with tooltip `composer.specialist.locked`; on select:

```rust
    let pick_specialist = move |id: String| {
        let Some(sid) = active_session.get() else { return; };
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "frameId": sid, "id": id })).unwrap();
            if invoke_checked("set_session_specialist", arg).await.is_ok() {
                let arg = to_value(&serde_json::json!({ "frameId": sid })).unwrap();
                let v = invoke("get_session_specialist", arg).await;
                session_specialist.set(serde_wasm_bindgen::from_value::<Option<Specialist>>(v).ok().flatten());
            }
        });
    };
```

Header badge: next to the session title render `{move || session_specialist.get().map(|s| view! { <span class="session-specialist">{s.name}</span> })}` (reuse an existing chip/badge class; add a minimal `.session-specialist` rule in `ui/src/styles/chat.css` if none fits).

- [ ] **Step 3: Compile + host tests**

Run: `cd ui && cargo test 2>&1 | tail -3 && cargo check --target wasm32-unknown-unknown 2>&1 | tail -1`
Expected: pass / clean.

- [ ] **Step 4: Commit**

```bash
git add ui/src/main.rs ui/src/i18n.rs ui/src/styles/chat.css
git commit -m "feat(ui): per-session specialist picker

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 8: Playwright coverage; PR 2

**Files:**
- Modify: `ui-tests/tests/mock-tauri.ts` (stateful specialists mock)
- Modify: `ui-tests/tests/ui.spec.ts`

- [ ] **Step 1: mock** — add to `tauriMock()` state and the `invoke` switch (the `plain()` helper from the vision fix already exists):

```ts
  let mockSpecialists: any[] = [
    { id: "reviewer", name: "Reviewer", icon: "review", color: "clay", description: "", instructions: "rubric", model_id: "", skills: [], connectors: [], builtin: true },
  ];
  let sessionSpecialists: Record<string, string> = {};
```

```ts
          case "list_specialists":
            return mockSpecialists;
          case "save_specialist_cmd": {
            const spec = plain(arg("spec") ?? {});
            if (!spec.id) { spec.id = `sp${mockSpecialists.length}`; spec.builtin = false; }
            mockSpecialists = mockSpecialists.some((s) => s.id === spec.id)
              ? mockSpecialists.map((s) => (s.id === spec.id ? { ...s, ...spec, builtin: s.builtin, instructions: s.builtin ? s.instructions : spec.instructions } : s))
              : [...mockSpecialists, spec];
            return mockSpecialists;
          }
          case "remove_specialist": {
            const id = arg("id");
            if (mockSpecialists.find((s) => s.id === id)?.builtin) throw new Error("Built-in specialists cannot be removed.");
            mockSpecialists = mockSpecialists.filter((s) => s.id !== id);
            return mockSpecialists;
          }
          case "set_session_specialist":
            sessionSpecialists[arg("frameId")] = arg("id");
            return null;
          case "get_session_specialist":
            return mockSpecialists.find((s) => s.id === sessionSpecialists[arg("frameId")]) ?? null;
```

- [ ] **Step 2: specs** (append to `ui.spec.ts`; follow the file's existing settings-navigation helpers)

```ts
test("specialists page lists builtin reviewer without a delete affordance and saves a custom specialist", async ({ page }) => {
  await page.goto("/?mock=1");
  await openSettingsSection(page, "specialists"); // follow the pattern used by the models tests to open settings + section
  await expect(page.getByText("Reviewer")).toBeVisible();
  await page.getByText("Add specialist").click();
  await page.getByText("Write from scratch").click();
  await page.getByLabel("Name").fill("Paper hunter");
  await page.getByRole("button", { name: /Save/ }).click();
  await expect(page.getByText("Paper hunter")).toBeVisible();
  // builtin row: open it and verify instructions are disabled + no remove button
  await page.getByText("Reviewer").click();
  await expect(page.getByLabel("Instructions")).toBeDisabled();
  await expect(page.getByRole("button", { name: /Remove/ })).toHaveCount(0);
});

test("new session can pick a specialist and it locks after the first message", async ({ page }) => {
  await page.goto("/?mock=1");
  // create specialist via mock state through the settings flow (as above), then:
  await page.getByText("New session").click();
  await openSessionOptions(page); // the ⚙/options popover helper used by existing compute tests
  await page.getByText("Specialist").click();
  await page.getByText("Paper hunter").click();
  await expect(page.locator(".session-specialist")).toHaveText("Paper hunter");
});
```

Adjust selectors/helpers to the file's actual helper names while implementing (the spec file already has helpers for opening settings and the composer options popover — reuse, do not reinvent).

- [ ] **Step 3: Run**

Run: `cd ui-tests && npx playwright test -g "specialist" --reporter=line 2>&1 | tail -4`
Expected: 2 passed.

- [ ] **Step 4: Commit and open PR 2**

```bash
git add ui-tests/tests/mock-tauri.ts ui-tests/tests/ui.spec.ts
git commit -m "test(ui): playwright coverage for specialists

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
git push
gh pr create --base main --title "feat: specialists UI (专家系统 PR 2/3)" --body "Specialists settings page + per-session picker per docs/superpowers/specs/2026-07-09-specialists-design.md. Depends on PR 1.

🤖 Generated with [Claude Code](https://claude.com/claude-code)"
```

---

### Task 9: `save_specialist` agent tool

**Files:**
- Create: `src-tauri/src/specialist_tool.rs`
- Modify: `src-tauri/src/lib.rs` (`mod specialist_tool;`; register at the agent-build block next to `RunInContextTool`)

**Interfaces:**
- Consumes: `specialists::upsert`.
- Produces: Registry tool `save_specialist` `{name, description?, instructions, model_id?, skills?, connectors?}` — always **creates** (never edits, so builtin rows are unreachable from chat).

- [ ] **Step 1: Write the failing test** (bottom of the new file)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use wisp_tools::Tool;

    struct NoEnv(std::path::PathBuf);
    #[async_trait::async_trait]
    impl wisp_tools::ToolEnv for NoEnv {
        fn project_root(&self) -> &std::path::Path { &self.0 }
        async fn confirm(&self, _m: &str) -> bool { true }
        async fn emit(&self, _e: wisp_tools::ToolEvent) {}
    }

    #[tokio::test]
    async fn creates_a_specialist_and_never_touches_builtin() {
        let tmp = std::env::temp_dir().join(format!("wisp_sptool_{}.sqlite", uuid::Uuid::new_v4()));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        let tool = SaveSpecialistTool { store: store.clone() };
        let env = NoEnv(std::env::temp_dir());
        let r = tool
            .run(&serde_json::json!({"name": "Reviewer", "instructions": "custom"}), &env)
            .await;
        assert!(r.success, "{}", r.content);
        // Same display name is fine — it created sp1, not the builtin.
        let reviewer = crate::specialists::get(&store, "reviewer").await.unwrap();
        assert_eq!(reviewer.instructions, crate::review::REVIEWER_RUBRIC);
        assert!(crate::specialists::get(&store, "sp1").await.is_some());
        let _ = std::fs::remove_file(&tmp);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p wisp-tauri specialist_tool 2>&1 | tail -3`
Expected: compile error — module missing.

- [ ] **Step 3: Implement**

```rust
//! `save_specialist` — lets the agent create a specialist from a chat
//! conversation ("Chat with Claude" creation flow). Create-only: editing and
//! deletion stay in the Settings UI, which keeps builtin rows unreachable.

use async_trait::async_trait;
use serde_json::{json, Value};
use wisp_llm::ToolSchema;
use wisp_store::Store;
use wisp_tools::{Tool, ToolEnv, ToolResult};

pub struct SaveSpecialistTool {
    pub store: Store,
}

fn str_arg(args: &Value, key: &str) -> String {
    args.get(key).and_then(|v| v.as_str()).unwrap_or_default().trim().to_string()
}

fn list_arg(args: &Value, key: &str) -> Option<Vec<String>> {
    args.get(key)?.as_array().map(|a| {
        a.iter().filter_map(|v| v.as_str()).map(str::to_string).collect()
    })
}

#[async_trait]
impl Tool for SaveSpecialistTool {
    fn name(&self) -> &str {
        "save_specialist"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "save_specialist",
            "Create a new specialist (agent persona) from this conversation: a name, \
             instructions appended to the base prompt, an optional bound model id, and \
             optional skill/connector whitelists. Use after interviewing the user about \
             what the specialist is for. Creates only — never edits existing specialists.",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Display name, e.g. 'Release notes writer'" },
                    "description": { "type": "string", "description": "One-line summary shown in settings (not in the prompt)" },
                    "instructions": { "type": "string", "description": "Persona instructions appended to the base system prompt" },
                    "model_id": { "type": "string", "description": "Model profile id to bind; omit to follow the active model" },
                    "skills": { "type": "array", "items": {"type": "string"}, "description": "Skill-name whitelist; omit to inherit project settings" },
                    "connectors": { "type": "array", "items": {"type": "string"}, "description": "Connector/MCP whitelist; omit to inherit" }
                },
                "required": ["name", "instructions"]
            }),
        )
    }
    fn preview(&self, args: &Value) -> String {
        str_arg(args, "name")
    }

    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        let name = str_arg(args, "name");
        if name.is_empty() {
            return ToolResult::fail("save_specialist error: 'name' is required");
        }
        let spec = crate::specialists::Specialist {
            id: String::new(), // create-only
            name,
            icon: "review".into(),
            color: "clay".into(),
            description: str_arg(args, "description"),
            instructions: str_arg(args, "instructions"),
            model_id: str_arg(args, "model_id"),
            skills: list_arg(args, "skills"),
            connectors: list_arg(args, "connectors"),
            builtin: false,
        };
        match crate::specialists::upsert(&self.store, spec).await {
            Ok(list) => {
                let created = list.iter().rev().find(|s| !s.builtin).cloned();
                ToolResult::ok(format!(
                    "Created specialist '{}' (id {}). The user can edit it under Settings → Specialists.",
                    created.as_ref().map(|s| s.name.as_str()).unwrap_or("?"),
                    created.as_ref().map(|s| s.id.as_str()).unwrap_or("?"),
                ))
            }
            Err(e) => ToolResult::fail(format!("save_specialist error: {e}")),
        }
    }
}
```

Register in `lib.rs` at the agent-build block (next to `RunInContextTool`):

```rust
        agent.add_tool(Box::new(specialist_tool::SaveSpecialistTool {
            store: state.store.clone(),
        }));
```

And `mod specialist_tool;` in the module list.

- [ ] **Step 4: Run tests**

Run: `cargo test -p wisp-tauri 2>&1 | grep -E 'test result|FAILED' | head -4`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/specialist_tool.rs src-tauri/src/lib.rs
git commit -m "feat(specialists): save_specialist tool for chat-driven creation

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 10: "Chat with Claude" entry; PR 3

**Files:**
- Modify: `ui/src/main.rs` (the Add specialist ▾ menu's second item from Task 6)
- Modify: `ui/src/i18n.rs`

**Interfaces:**
- Consumes: existing `new_session` command + composer send path; `save_specialist` tool from Task 9.

- [ ] **Step 1: i18n key**

```
"specialists.chat_prompt" => "I want to create a new specialist. Interview me about: what it's for, its tone and working style, which skills and data sources it needs, and what class of model suits it. Then call save_specialist to save it." (我想创建一个新专家。请先访谈我:它的用途、语气和工作风格、需要哪些技能和数据源、适合什么档次的模型。然后调用 save_specialist 保存。)
```

- [ ] **Step 2: wire the menu item** — "Chat with Claude" closes settings, creates a session, and sends the template as the first message (reuse the exact flow of the composer's send handler: `invoke("new_session")` → set active → `send_message` with the template text):

```rust
    let chat_create_specialist = move |_| {
        show_settings.set(false);
        let loc = locale.get();
        spawn_local(async move {
            let v = invoke("new_session", JsValue::UNDEFINED).await;
            let Ok(sid) = serde_wasm_bindgen::from_value::<String>(v) else { return; };
            // route the UI to the new session, then send the interview prompt
            open_session(sid.clone()); // the existing helper the sidebar uses
            let arg = to_value(&tauri_args::send_message_args(&Some(sid), &t(loc, "specialists.chat_prompt"), &[])).unwrap();
            let _ = invoke_checked("send_message", arg).await;
        });
    };
```

(Adapt `open_session` / `send_message_args` to the actual helper names in `main.rs` / `tauri_args` — both flows already exist for the sidebar "New session" button and the composer send; reuse them rather than duplicating.)

- [ ] **Step 3: Playwright** (append to the specialists spec from Task 8)

```ts
test("chat-with-claude creation opens a new session with the interview prompt", async ({ page }) => {
  await page.goto("/?mock=1");
  await openSettingsSection(page, "specialists");
  await page.getByText("Add specialist").click();
  await page.getByText("Chat with Claude").click();
  // settings closed, a session is active, and send_message was invoked with the template
  const calls = await page.evaluate(() => (window as any).__skillInvokeLog.filter((c: any) => c.cmd === "send_message"));
  expect(calls.length).toBeGreaterThan(0);
});
```

- [ ] **Step 4: Full sweep**

Run: `cargo test --workspace`, `cd ui && cargo test && cargo check --target wasm32-unknown-unknown`, `cd ui-tests && npx playwright test --reporter=line | tail -3`
Expected: all green.

- [ ] **Step 5: Commit and open PR 3**

```bash
git add ui/src/main.rs ui/src/i18n.rs ui-tests/tests/ui.spec.ts
git commit -m "feat(specialists): chat-driven creation entry

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
git push
gh pr create --base main --title "feat: specialists chat creation (专家系统 PR 3/3)" --body "save_specialist tool + Chat-with-Claude entry per docs/superpowers/specs/2026-07-09-specialists-design.md. Depends on PR 1/2.

🤖 Generated with [Claude Code](https://claude.com/claude-code)"
```
