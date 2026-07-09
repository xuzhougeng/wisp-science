# Specialists (专家系统) — Design

**Date:** 2026-07-09
**Status:** Approved
**Reference:** Claude Science's Specialists (registry + per-session selector + built-in Reviewer); wisp adaptation decided in-session.

## Goal

User-definable agent personas ("specialists"): a named instruction set plus a
skill/MCP subset and a directly-bound model, selectable per session. The
existing one-shot session review becomes the built-in **Reviewer** specialist.
Specialists can be created by hand (form) or drafted through a chat
conversation (`save_specialist` tool).

## Architecture position

- **ModelProfile** stays the supply side: connection + capability
  (`supports_vision` etc.).
- **Specialist** is the usage side: prompt + skill/MCP subset + **directly
  bound model** (`model_id`). There is **no intermediate assignment/slot
  layer** — a specialist *is* the usage descriptor. If bulk re-pointing ever
  matters, `model_id` can later accept an alias form (e.g. `slot:cheap`)
  without breaking stored data.
- **vision** remains the one special-purpose assignment (`VISION_KEY`,
  unchanged): `view_image` is a capability route, not a persona.

Explicitly out of scope for v1 (recorded, not forgotten): mid-turn delegation
to specialists (sub-agent orchestration), per-specialist tool exclusions /
thinking toggles, specialist import/export as files.

## 1. Data model

New module `src-tauri/src/specialists.rs`:

```rust
pub struct Specialist {
    pub id: String,            // "reviewer" (builtin) | "s1", "s2", ... (fresh_id pattern)
    pub name: String,
    pub icon: String,          // name from the UI's built-in icon set
    pub color: String,         // color token
    pub description: String,   // shown to the user only; never enters the prompt
    pub instructions: String,  // appended to the base system prompt
    pub model_id: String,      // "" = follow the active model
    pub skills: Option<Vec<String>>,      // None = inherit project skill config; Some = whitelist
    pub connectors: Option<Vec<String>>,  // None = inherit; Some = connector/MCP whitelist
    pub builtin: bool,         // Reviewer: not deletable, instructions read-only
}
```

- **Storage:** sqlite settings key `specialists` (JSON array) — same pattern
  as `model_profiles`. Builtin Reviewer is materialized into the list on first
  read (`ensure`-style), so user edits to its `model_id`/`skills` persist like
  any other row; `builtin: true` gates deletion and instruction edits in both
  backend commands and UI.
- **Prompt composition** follows the reference app's replace-identity /
  inherit-working-style split: wisp's base system prompt stays intact;
  `instructions` is appended as an identity section
  (`\n\n## Specialist: {name}\n{instructions}`).
- **Model resolution:** `specialist_config(store, &specialist)` — resolve
  `model_id` against profiles; empty or dangling → fall back to the active
  model. Sits next to `vision_config()`.

## 2. Session persona mechanics

Effective at the existing Agent construction point (`src-tauri/src/lib.rs`,
session-runtime build around line 1653). No `agent_loop` changes.

- Per-session selection stored as setting `frame_specialist:<frame_id>`.
  Selectable from the composer's session-options menu **after session creation
  and before the first message**; locked once the session has messages
  (switching personas mid-context invites confusion).
- When a specialist is set, Agent construction applies:
  - **Prompt:** append the identity section after `seed_system_prompt`.
  - **Skills:** `active_skill_index()` result further filtered through
    `SkillIndex::filtered_by_names(specialist.skills)` (function already used
    by `mcp_bridge`).
  - **Connectors/MCP:** `wire_python_and_mcp` gains an optional whitelist
    parameter; bio domains and custom connections outside it are skipped
    (skip machinery already exists — only the data source is new).
  - **Model:** provider config comes from `specialist_config` instead of the
    active profile.

## 3. Reviewer upgrade (Request review)

- Builtin specialist `id="reviewer"`, `instructions = REVIEWER_RUBRIC`
  (`src-tauri/src/review.rs`), `model_id: ""`, `builtin: true`.
- `review_session` command changes only its config source: instructions from
  the reviewer specialist row, model from `specialist_config`. The one-shot
  flow (transcript serialization, caps, output rendering) is untouched.
- Changing the review model = editing the builtin Reviewer's model binding in
  the Specialists page. Custom review rules = duplicate Reviewer into a custom
  specialist (v1 keeps builtin instructions read-only).
- The existing "Request review" UI entry is unchanged.

## 4. Creation flows

- **Form** (Specialists settings page): fields mirror the data model. No
  user-facing "Agent ID" field — ids are auto-assigned.
- **Chat generation:** new `save_specialist` tool registered in the Registry
  (subject to the per-tool approval gate). The "Chat with Claude" menu item
  opens a new session pre-filled with a user message template: interview me
  about the specialist I want (purpose, tone, which skills/data sources, what
  class of model), then call `save_specialist`. The result appears in the
  Specialists page for further editing. No dedicated wizard UI.

## 5. UI (Leptos)

- New **Specialists** settings page: list grouped Built-in / Custom; edit
  form; "Add specialist ▾" → Write from scratch / Chat with Claude.
- Composer session-options popover gains a **Specialist** submenu:
  None / each specialist / Create new…. The active specialist's icon + name
  show next to the session title.
- Specialist form includes a model dropdown (default "follow active model").
- Models page is **not** touched (no assignments section).

## 6. Testing

- Rust: specialist CRUD round-trip; model resolution fallback (empty id,
  dangling id → active); skill/connector whitelist filtering applied to the
  built agent; builtin reviewer cannot be deleted and its instructions cannot
  be modified via `save_specialist`/commands.
- Playwright (mock): create specialist → new session selects it →
  `send_message` carries the specialist; Request review resolves the reviewer
  specialist; builtin rows render without a delete affordance.

## Delivery plan

Three PRs:

1. Backend: `specialists.rs` (store + commands + builtin Reviewer), agent
   construction hooks, `review_session` re-source, tests.
2. UI: Specialists settings page + composer session selector + Playwright
   coverage.
3. `save_specialist` tool + "Chat with Claude" entry.
