# Research Workbench Control Plane

## Goal

Make wisp-science a project-level research workbench rather than a local chat shell with occasional SSH commands. The user should see one research project even when compute happens across local Python, WSL, SSH Linux servers, GPU hosts, SLURM clusters, literature tools, and local figure polishing.

## Core Model

### Project

A research problem and its accumulated state: conversations, decisions, data assets, runs, artifacts, papers, and notes. A project is not identical to one local directory or one remote directory.

### ExecutionContext

A place where commands can execute or tools can be called. Initial context families:

- `local`: current host filesystem and shell.
- `wsl:<distro>`: persistent WSL distribution context on Windows.
- `ssh:<alias>`: host from SSH config or user registry.
- Future: `slurm:<cluster>`, `modal:<workspace>`, `literature:<provider>`, `mcp:<server>`.

Each context should expose capabilities: OS, arch, CPU count, memory if available, GPU summary if available, scheduler if available, conda/mamba/module hints, default workdir, data roots, and whether internet access appears available.

### DataAsset

A data reference. It can be local, WSL, SSH remote, object-store, or external URL. Large omics data should usually be represented as references rather than copied locally. DataAsset records should include role, URI/path, optional checksum, size, origin, and produced_by_run_id when applicable.

### Run

One reproducible unit of work. A Run records the context, command/script/workflow, inputs, environment/probe snapshot, stdout/stderr tails, status, scheduler/job handle, exit code, and produced artifacts.

### Artifact

A consumable output: figure, table, report, model, PDB/mmCIF, notebook, markdown, literature summary, or decision note. Artifacts may live locally or remotely, but the project index should show them uniformly.

## Non-Goals For The First Implementation

- Do not build a full VS Code Remote clone.
- Do not require a daemon on remote hosts in v0.
- Do not sync entire project directories by default.
- Do not require real servers in tests.
- Do not turn every chat message into a graph node; start with explicit decisions/artifacts/runs.

## Milestones

### M1: ExecutionContext v0

Persist contexts, import SSH aliases, detect WSL distros, run capability probes, show contexts to the agent and UI.

### M2: Run Manager v1

Persist runs/jobs, add status lifecycle, record stdout/stderr/exit status, support cancellation, and harvest declared outputs.

### M3: Workspace Manifest v1

Create and maintain a typed project layout and tool APIs that save/register scripts, data assets, figures, results, reports, literature, and docs into consistent places.

### M4: Research Graph v0

Add linkable nodes and edges for question, decision, data asset, run, artifact, and paper. Use this to answer provenance questions.

## Acceptance Scenario

A user creates one project, registers an omics SSH server and a GPU SSH server, registers data or asks wisp to download data remotely, runs an omics analysis on the omics host, harvests QC reports and result tables, runs a GPU structure analysis on the GPU host using results from the omics run, then creates final figures locally. The project timeline shows all steps as one coherent research history.
