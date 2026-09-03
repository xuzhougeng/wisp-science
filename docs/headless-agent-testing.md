# Headless agent testing

Wisp has two complementary headless interfaces:

- `wisp-science eval` runs repeatable agent conformance and regression suites.
- `wisp-science rpc` runs a long-lived agent over a versioned JSONL stdin/stdout protocol.

Both exercise the production `Agent` loop and tool registry, including the Run
control plane (`run_in_context`, `monitor_run`, `get_run`, `cancel_run`,
harvest/cleanup/transfer). Interactive/`run`/`rpc` open `.wisp/wisp.sqlite` in
the project (override with `WISP_STORE`) so long jobs wait through
`monitor_run` instead of shell `sleep` polling. SSH/WSL contexts in that store
are used when present; the desktop app's host registry is a separate database
unless `WISP_STORE` points at it.

The default eval suite uses a deterministic scripted provider, temporary
workspaces, and fixture MCP/subagent boundaries, so it requires no API key,
network, SSH host, GPU, scheduler, Python, or R installation.

## Offline evaluation

Run the built-in suite:

```bash
cargo run -p wisp-cli -- eval \
  --artifacts target/headless-agent-eval \
  --save target/headless-agent-eval/report.json
```

The suite covers reads and exact edits, shell execution, persisted local Runs
(`run_in_context` / `monitor_run` instead of shell `sleep` polling), persistent
Python and R runtime cells, approval denial, skills, deferred MCP discovery,
read-only subagent delegation, resume without tool replay, session restart,
queued guidance, cancellation, vision fallback, manual compaction, plan-mode
gating, and project path containment.

Useful selection and stress controls:

```bash
# Select by stable case id or require a tag.
cargo run -p wisp-cli -- eval --case targeted-edit --tag filesystem

# Detect nondeterminism and exercise concurrent workspace isolation.
cargo run -p wisp-cli -- eval --repeat 20 --parallel 4

# Preserve only failed temporary projects for investigation.
cargo run -p wisp-cli -- eval \
  --keep-failed-workspace --artifacts target/headless-agent-eval
```

Every trajectory is JSONL with schema `wisp.agent-trajectory.v1`. It includes
the full provider requests, messages, tool call IDs, parsed tool arguments,
tool results, approvals, compaction events, and usage. The JSON summary uses
`wisp.agent-eval-report.v1`.

### Budgets and baselines

The runner fails when a selected case fails, a budget is exceeded, the pass
rate is below the requested threshold, or a baseline regression crosses a
configured threshold:

```bash
cargo run -p wisp-cli -- eval \
  --max-tool-calls 8 \
  --max-input-tokens 200000 \
  --max-duration-ms 15000 \
  --max-cost-microusd 50000 \
  --input-cost-microusd-per-million 3000000 \
  --output-cost-microusd-per-million 15000000

cargo run -p wisp-cli -- eval --save baseline.json
cargo run -p wisp-cli -- eval \
  --compare baseline.json \
  --max-token-regression-percent 10 \
  --max-round-regression 2 \
  --save current.json
```

Cost rates are explicit integer micro-USD per million tokens; offline defaults
are zero. Cached input is excluded from the billable input estimate.

Use `--mode live` to run the same declarative suite against the configured
provider. Live mode requires the normal `WISP_API_KEY`, `WISP_PROVIDER`, and
`WISP_MODEL` environment variables. For a text-only primary model, set
`WISP_VISION_MODEL` (and optionally `WISP_VISION_PROVIDER`,
`WISP_VISION_API_URL`, `WISP_VISION_API_KEY`) so `view_image` is described by
a dedicated vision model instead of being sent to the chat model. Scripted
steps are ignored in live mode; live suites should use tolerant semantic
assertions rather than exact prose.

### Memory and compaction dataset

`crates/wisp-cli/eval-suites/memory-v1.yaml` is a deterministic offline
dataset for the memory mechanism. It verifies three axes:

1. **Compression without forgetting.** One scenario per compaction trigger
   mode — `auto` (80% boundary before a model call), `manual` (`/compact`
   action), and `overflow` (a scripted provider context-overflow error forces
   one recovery compaction and a retry). Each seeds early facts buried under
   bulk filler, requires exactly that strategy, requires real shrinkage via
   `max_compaction_ratio_percent`, and asserts through `final_request_contains`
   / `final_request_not_contains` that the post-compaction request still
   carries the early facts, the `[context summary checkpoint]`, the
   `wisp-history:` archive pointer, and the recent tail, while the folded
   filler is gone.
2. **Retrieval correctness.** `search_memory` scenarios over seeded
   `.wisp/memory/*.md` notes: multi-query fan-out that must return the right
   chunk and exclude distractors, CJK matching, an explicit empty-result miss,
   and the setting gate (the tool is not registered while memory is disabled).
3. **Stage loading.** Session-start rules (`AGENTS.md` + `.wisp/WISP.md` in
   the system prompt), per-turn ephemeral host injection reaching the provider
   request, injections *not* surviving a restart as durable history, and
   durable project notes remaining retrievable after a restart.

```bash
cargo run -p wisp-cli -- eval --suite crates/wisp-cli/eval-suites/memory-v1.yaml
```

The suite also runs inside `cargo test -p wisp-cli` so regressions surface in
the normal workspace test run. Case knobs added for this dataset:
`memory_enabled` registers the production `search_memory` tool, and
`runtime_injections` mirrors the host's per-turn global-memory injection.
Expectation knobs: `compaction_strategies` pins the exact ordered trigger
modes, and `final_request_contains` / `final_request_not_contains` assert on
the last provider request only — sharper than `request_contains` after a
compaction or restart. Scripted steps may fail deterministically with
`api_error: {status, body}` to exercise provider error paths.

### Cross-model compaction benchmark

The repository includes `crates/wisp-cli/eval-suites/live-compaction-v1.yaml`
for comparing models after production semantic compaction. It measures exact
task continuity in a realistic longitudinal-study handoff. Before compaction,
the model must discover and read study inputs, calculate cohort results with
Python, write an analysis artifact, and apply an authoritative correction with
the edit tool. After compaction it must re-ground itself from the artifacts and
write a final report with the correct decision, threshold, exclusion, completed
work, and next action. The scenario must actually compact to at most 80% of its
original token estimate and finish through the normal `attempt_completion` tool.

Configure one provider endpoint/key, then repeat `--model` for every model that
endpoint exposes:

```bash
export WISP_API_KEY=<provider-key>
export WISP_PROVIDER=openai
export WISP_API_URL=https://api.example.com/v1

cargo run -p wisp-cli -- eval \
  --mode live \
  --suite crates/wisp-cli/eval-suites/live-compaction-v1.yaml \
  --model model-a \
  --model model-b \
  --model model-c \
  --parallel 1 \
  --artifacts target/live-compaction \
  --save target/live-compaction/report.json
```

Keep `--parallel 1` for an initial comparison so provider rate limits and
latency are comparable. Add `--repeat 3` or more after the smoke run to expose
variance. The report's `model_summaries` array contains per-model pass rate,
token usage, cost, latency, compaction count, aggregate before/after token
estimates, and `compaction_ratio_percent`. Every scenario also records its
model and individual compaction measurements. Trajectory filenames include the
model, case, and repetition so matrix runs never overwrite one another.

Models in one invocation share the provider kind, URL, API key, and request
settings. Run separate invocations when comparing models hosted by different
providers, then compare their saved reports independently.

### Live memory dataset

`crates/wisp-cli/eval-suites/live-memory-v1.yaml` runs the memory mechanism
against a real configured model. It covers the same three axes as the offline
`memory-v1` dataset, with assertions that still work without a scripted
provider snapshot:

1. **Compression without forgetting.** Manual `/compact` and auto-compaction
   bury locked identifiers (`QC_THRESHOLD=0.047`, `COHORT=WISP-HCC-2024-G`)
   only in folded assistant turns. After production semantic compaction the
   model must report those values without reading the archive. The scenario
   must actually compact, shrink to at most 80% of its original token
   estimate, and finish through `attempt_completion`.
2. **Retrieval.** Facts exist only in `.wisp/memory/*.md`. The model must
   call `search_memory` and use the retrieved chunk, including a CJK query
   and durable notes that remain retrievable after `restart`. Distractor
   studies must not leak into the answer.
3. **Stage loading.** `AGENTS.md`, `.wisp/WISP.md`, and a per-turn
   `<global_memory>` injection must appear in the live completion.

Overflow recovery, the memory-off tool gate, and `final_request_*` body
checks stay in the offline dataset: a live provider cannot script a
deterministic 400, and live mode does not record request snapshots.

```bash
export WISP_API_KEY=<provider-key>
export WISP_PROVIDER=openai
export WISP_API_URL=https://api.example.com/v1

cargo run -p wisp-cli -- eval \
  --mode live \
  --suite crates/wisp-cli/eval-suites/live-memory-v1.yaml \
  --model model-a \
  --parallel 1 \
  --artifacts target/live-memory \
  --save target/live-memory/report.json
```

Keep `--parallel 1` for an initial smoke so rate limits are comparable. Add
`--repeat 3` after that to expose variance. Cargo tests validate the suite
and execute every case through the production loop with a scripted provider;
they never require an API key.

The same long-context scenario is also bundled as an Example-project demo
(`seed/manifest_memory_01_long_context.json`). Open **Example project**,
right-click **Long-context memory demo**, and choose **Copy to a project…**
to materialize the transcript (and `.wisp/memory` notes) in a real workspace.
The transcript is a complete GSE153250 ESR1-knockdown RNA-seq analysis
session recorded live with the wisp CLI (58 turns, ~104K estimated tokens;
see `scripts/export_memory_demo.py`). The opening assistant turn locks the
analysis decision (`GENE_FILTER`, `PRIMARY_CONTRAST`, `FDR_CUTOFF`); the
session then runs QC, PCA, exploratory DE, sensitivity checks, figures, and
report drafts, so a manual `/compact` installs the semantic checkpoint and
folds the opening away whenever the configured context window is ~110K or
smaller (the fold gate is 60% of the window). Continue with a live model:
`/compact`, then ask what the first
answer locked, or ask which sample was flagged for low assignment (`siNT_1`)
to exercise `search_memory` against the confirmed notes — including a
distractor ChIP-seq pilot (`GSE180386`).

### Custom suites

Pass a YAML or JSON file with `--suite`. The top-level schema is
`wisp.agent-eval-suite.v1`:

```yaml
schema: wisp.agent-eval-suite.v1
id: project-smoke-v1
defaults:
  timeout_ms: 15000
  max_rounds: 8
cases:
  - id: inspect-config
    description: Read the fixture and report its mode.
    tags: [filesystem, smoke]
    prompt: Read config.toml and report mode.
    files:
      config.toml: "mode = \"safe\"\n"
    allowed_tools: [read, attempt_completion]
    script:
      - tool_calls:
          - id: read-1
            name: read
            arguments: {path: config.toml}
      - tool_calls:
          - id: done-1
            name: attempt_completion
            arguments: {result: "mode=safe"}
    expect:
      outcome: success
      completion_contains: [mode=safe]
      required_tools: [read, attempt_completion]
      forbidden_tools: [write, edit, shell]
      tool_order: [read, attempt_completion]
      tool_args:
        - {name: read, pointer: /path, equals: config.toml}
```

Fixture paths must be relative and remain under the temporary project.
`allowed_tools` is an exact capability allowlist. Binary fixtures use
`base64_files`. Multi-turn lifecycle scenarios use `actions` (`send`, `resume`,
`compact`, and `restart`).

## One-shot JSONL

`run --output jsonl` emits `wisp.agent-event.v1`. Every line contains a
monotonic `sequence`, `session_id`, `turn_id`, and event `type`. Tool events
include the provider call ID and full parsed arguments when available. Setup
diagnostics go to stderr, leaving stdout machine-readable.

```bash
cargo run -p wisp-cli -- run --output jsonl "Inspect this project"
```

This mode is intentionally non-interactive: an approval request is emitted and
denied. Use RPC when the controller must answer approvals or cancel a turn.

## Bidirectional RPC

Start a persistent process with the normal provider configuration:

```bash
cargo run -p wisp-cli -- rpc
```

Commands are one JSON object per stdin line. All commands must use
`wisp.agent-rpc.v1` and a caller-defined unique `id`:

```json
{"schema":"wisp.agent-rpc.v1","id":"turn-1","type":"prompt","prompt":"Inspect README.md"}
{"schema":"wisp.agent-rpc.v1","id":"ping-1","type":"ping"}
{"schema":"wisp.agent-rpc.v1","id":"cancel-1","type":"cancel"}
{"schema":"wisp.agent-rpc.v1","id":"approval-1","type":"approval_response","approval_id":"<event approval_id>","approved":false,"feedback":"Do not mutate files"}
{"schema":"wisp.agent-rpc.v1","id":"shutdown-1","type":"shutdown"}
```

The process first emits `ready`. A prompt produces `turn_started`, streaming
message/text/reasoning/tool/usage events, then exactly one `turn_completed`.
While a turn is active the controller may send `ping`, `cancel`,
`approval_response`, or `shutdown`; a second prompt is rejected. Every event
has the schema, sequence, process `session_id`, and relevant `command_id`.

When a tool needs confirmation, Wisp emits:

```json
{"schema":"wisp.agent-rpc.v1","type":"approval_required","approval_id":"...","message":"Run tool 'write'?", "command_id":"turn-1"}
```

The agent remains suspended without blocking the command reader until a
matching `approval_response`, cancellation, shutdown, or stdin closure. Unknown
schemas and malformed input produce `protocol_error` events without terminating
the process.

## CI contract

The offline suite runs on Linux, macOS, and Windows. Tests must never add a
dependency on a real remote host, scheduler, API key, or language runtime.
Platform integrations should use fake command runners and parsing tests; live
provider suites belong in separately credentialed, non-blocking jobs.
