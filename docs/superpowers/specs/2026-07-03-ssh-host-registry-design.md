# SSH host registry + agent awareness

Resolves [#30](https://github.com/xuzhougeng/wisp-science/issues/30) (scope A).

## Problem

The agent has no idea which remote machines the user can reach, so it can't
choose to run heavy jobs on them and it looks for remote files on the local
box ("file not found"). wisp ships `remote-compute-ssh` / `remote-compute-modal`
/ `compute-env-setup` skills, but those call a `host.compute` control-plane that
wisp does **not** have. What wisp *does* have is a `shell` tool that can already
run `ssh <alias> "<cmd>"`. The gap is a **registry of hosts** plus **telling the
agent they exist**.

## Goals

- Register SSH hosts (from `~/.ssh/config` or typed by hand), persist them.
- Make the running agent aware of the hosts and how to reach them.
- List/manage hosts in the right sidebar; add via a composer button.

## Non-goals (deferred — scope B/C, much larger)

- Embedded terminal, tmux, live command streaming.
- A dedicated `run_on_host` tool with a per-command approval modal.
- Any `host.compute` control-plane / job-scheduler abstraction.

## Data model

```rust
struct SshHost {
    alias: String,              // required; unique key
    user: Option<String>,       // Advanced override; else from ~/.ssh/config
    port: Option<u16>,          // Advanced override; else 22 / config
    identity_file: Option<String>,
    notes: Option<String>,      // "sbatch/qsub? conda? partition?" free text
}
```

Persisted as a JSON array under the existing `Store` settings key `ssh_hosts`.
No new DB table — reuses `get_setting` / `set_setting`.

## Backend — `src-tauri/src/ssh_hosts.rs` + 4 tauri commands

- `list_ssh_hosts() -> Vec<SshHost>` — read + parse the `ssh_hosts` setting.
- `add_ssh_host(host: SshHost) -> Vec<SshHost>` — upsert by `alias`, persist, return list.
- `remove_ssh_host(alias: String) -> Vec<SshHost>` — drop by alias, persist, return list.
- `list_ssh_config_aliases() -> Vec<String>` — parse `~/.ssh/config` `Host` lines,
  skip wildcard patterns (`*`, `?`), dedupe. **Pure function → TDD-covered.**

Registered in `generate_handler![]`.

## Agent awareness

New section in `SystemPrompt::assemble()`:

```
## Compute hosts

The user has these SSH hosts available. Run remote commands with the shell
tool: `ssh <alias> '<cmd>'`. Prefer them for heavy jobs; remote paths live on
the host, not the local box.

- <alias> — <user@alias:port> — <notes>
- ...
```

`SystemPrompt::new` gains a `hosts: &[SshHost]` param (empty slice → section
omitted). `seed_system_prompt` loads hosts from the store and passes them.
Seeded once at session start (**mid-session adds apply on the next session** —
acceptable for MVP; runtime injection via `ctx.inject_user` is the upgrade path).

## UI

- **Composer**: a new servers-icon button in `.composer-tools` (next to `+`),
  opening a small popover: **Add SSH host…**, the current host list, and
  "Manage in panel" (opens the sidebar tab).
- **Add SSH host panel** (drawer, mirrors the reference): a `~/.ssh/config`
  alias dropdown *or* free-text alias, an Advanced section (user / port /
  identity file), and a notes textarea. Add button calls `add_ssh_host`.
- **Right sidebar**: new `RightTab::Hosts` listing each host (alias, connection,
  notes) with a remove button and an inline Add entry point.

## Testing

- `list_ssh_config_aliases` parser: TDD — `Host` lines parsed, wildcards skipped,
  multiple aliases on one line handled, deduped.
- `add`/`remove` upsert semantics: unit test on the pure list transform.
- Frontend: compile-check (wasm) + browser preview of the panel and sidebar tab.

## Risks / cleanup flagged (not done here)

- `remote-compute-ssh` / `remote-compute-modal` / `compute-env-setup` skills
  reference the absent `host.compute` control-plane; the agent could `use_skill`
  one and fail. Mitigation: the injected guidance routes it to `ssh <alias>` via
  shell. Removing/rewriting those 3 skills is a separate follow-up.
- No connectivity check on add — an unreachable alias is stored silently. A
  "Test connection" button is a possible later addition.
