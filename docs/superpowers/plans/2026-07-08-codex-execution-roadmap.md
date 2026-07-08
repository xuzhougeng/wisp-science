# Codex Execution Roadmap: Local/Remote Unified Research Workbench

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the research-workbench architecture into small, reviewable Codex tasks. Each task should be independently testable and should not require access to a real SSH host, WSL distro, GPU, SLURM cluster, or external network.

**Architecture:** Build a control plane in small layers: persistent execution contexts first, then bridges from existing SSH/WSL discovery, then probes, runs, harvest, workspace manifest, UI, and research graph. Every PR must leave existing commands compatible and add tests that run without real remote infrastructure.

**Tech Stack:** Rust, Tauri 2, sqlx SQLite, Leptos, Playwright, mocked command runners.

---

## Task Sequence

### PR 0: Add Repo-Level Agent Guidance

Add `AGENTS.md` at repo root with build/test commands, project layout, remote-compute constraints, and PR expectations.

Acceptance:

- `AGENTS.md` exists at repo root.
- It does not contain secrets or environment-specific paths.
- It tells Codex not to implement the whole roadmap in one PR.

Verification:

```bash
test -f AGENTS.md
git diff --name-only
```

Expected diff contains docs and prompt files only; no Rust or UI source files.

### PR 1: ExecutionContext Data Model And Store API

Add a typed execution context model in Rust, probably under `src-tauri/src/execution_contexts.rs` or a new crate if reuse is needed. Add idempotent SQLite tables or settings-backed storage for contexts.

Suggested model fields:

- `id`: e.g. `local`, `ssh:omics`, `wsl:Ubuntu`.
- `kind`: `local | ssh | wsl`.
- `label`.
- `created_at`, `updated_at`.
- `config_json`: non-secret config such as alias, distro, default workdir, data roots.
- `capabilities_json`: last probe result.
- `last_probe_at`, `last_probe_status`, `last_probe_error`.

Acceptance:

- Pure unit tests for ID parsing and serialization.
- Store roundtrip tests.
- Existing `ssh_hosts` behavior continues to pass.
- `cargo test --workspace` passes.

### PR 2: SSH Registry Becomes ExecutionContext-Backed

Bridge existing SSH host registry to the new context registry without breaking the existing UI commands.

Implementation notes:

- Keep current Tauri commands stable: `list_ssh_hosts`, `add_ssh_host`, `remove_ssh_host`, `list_ssh_config_aliases`, `import_ssh_config_hosts`.
- Internally, create/update corresponding `ssh:<alias>` contexts.
- Render the agent prompt from contexts rather than only from the legacy `ssh_hosts` setting.
- Do not store SSH private key contents.

Acceptance:

- Existing SSH host unit tests pass.
- New tests verify importing aliases creates SSH contexts.
- Agent prompt includes context IDs and warns that remote paths are not local paths.

### PR 3: WSL Context Discovery v0

Add a Windows-only WSL discovery command and context registration. Non-Windows builds should compile and return an empty or unsupported result.

Suggested Tauri commands:

- `list_wsl_distros() -> Vec<WslDistro>`.
- `import_wsl_contexts() -> Vec<ExecutionContext>`.

Implementation notes:

- Use `wsl.exe -l -q` on Windows.
- Add parser tests with fixture strings; do not require WSL in CI.
- Normalize IDs as `wsl:<distro>`.
- Do not yet implement a full persistent terminal; just register the context and default workdir/path mapping fields.

Acceptance:

- Parser handles CRLF, null bytes if present, blank lines, and default distro markers.
- CI passes on Linux/macOS/Windows without WSL.

### PR 4: Context Probe v0

Add a probe service that can collect capabilities for local, SSH, and WSL contexts. The runner must be mockable.

Probe fields:

- `os`, `arch`, `hostname`.
- `cpu_count`.
- `gpu_summary` from `nvidia-smi -L` when available.
- `scheduler`: detected from `sbatch`, `qsub`, or `bsub`.
- `python`, `conda`, `mamba`, `modulecmd` hints.
- `default_shell`, `home`, `pwd` if safe.

Acceptance:

- Unit tests use fake command outputs.
- Probe errors are stored without deleting previous good capabilities.
- Agent prompt summarizes capabilities compactly.

### PR 5: Run Manager Schema And Lifecycle v0

Add persisted runs and jobs. Do not implement a complex scheduler yet.

Suggested tables:

- `runs(id, project_id, frame_id, context_id, title, kind, status, command, script_path, input_refs_json, output_specs_json, created_at, started_at, ended_at, exit_code, stdout_tail, stderr_tail, remote_workdir, env_snapshot_json)`.
- `run_artifacts(id, run_id, artifact_id, role, created_at)`.

Statuses:

- `draft`, `submitted`, `running`, `succeeded`, `failed`, `cancelled`, `lost`.

Acceptance:

- Store roundtrip tests.
- Status transition tests.
- No UI required yet.

### PR 6: Direct Command Run Tool v0

Add an agent tool such as `submit_run` or `run_in_context` that creates a Run and executes a bounded command in `local`, `ssh:<alias>`, or `wsl:<distro>` context. This can internally use process execution but must write Run records.

Implementation notes:

- Keep the old `shell` tool for quick local commands.
- This tool should return a run ID and status.
- For v0, commands can be bounded by a timeout, but the model should be extensible to background jobs.
- Use safe quoting. Prefer argument vectors internally where possible.

Acceptance:

- Tests with fake runner.
- Manual smoke can run a local command and see a persisted run.
- Stop/cancel behavior is defined, even if limited.

### PR 7: Harvest v0 And Remote Artifact References

Add output specs and harvest behavior for small files. Large or remote-resident files should be indexed without forced download.

Suggested output spec fields:

- `glob`.
- `kind`: `table | figure | report | model | log | data | other`.
- `residency`: `local | remote | auto`.
- `max_file_mb`, `max_total_mb`.

Acceptance:

- Harvest registers artifacts for small files.
- Files over threshold are represented as remote references.
- Provenance/run linkage is stored.

### PR 8: Workspace Manifest v1

Add `.wisp/project.toml` or `.wisp/WISP.md` generation and enforce typed directories through helper functions.

Default layout:

```text
project/
  .wisp/
  data/raw/
  data/external/
  data/processed/
  analysis/scripts/
  analysis/notebooks/
  analysis/workflows/
  runs/
  results/tables/
  results/models/
  results/reports/
  figures/
  literature/
  docs/
```

Acceptance:

- New projects can initialize this layout.
- Existing projects are not destructively rearranged.
- `save_artifact` or equivalent helper can place files by kind.
- Addresses issue #59 at the tool/API level, not just via prompt text.

### PR 9: UI Surfaces v0

Add a minimal Contexts panel and Runs timeline. Do not build every future graph feature yet.

Acceptance:

- User can see local/SSH/WSL contexts and last probe status.
- User can see run status, context, start/end time, and produced artifacts.
- Playwright mock covers the new panels.

### PR 10: Research Graph v0

Add graph nodes and links for decisions, papers, data assets, runs, and artifacts.

Acceptance:

- Can record a decision with optional links to runs/artifacts/papers.
- Can answer or display "this artifact was produced by run X using data Y on context Z".
- Backward-compatible with existing artifacts/provenance.

## Execution Rule For Codex

Only give Codex one PR at a time. Each task prompt should explicitly say:

- Implement only this PR.
- Do not start later roadmap items.
- Keep command names backward-compatible unless the task says otherwise.
- Add tests.
- Run the verification commands and report exact results.
