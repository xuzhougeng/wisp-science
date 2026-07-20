# Dynamic Temporary Agent Delegation — Design

**Date:** 2026-07-20  
**Status:** Implemented
**Supersedes:** the fixed-template planning model described in
`docs/agent-delegation.md`  
**Reference implementation studied:**
[`can1357/oh-my-pi` at `39c95e5`](https://github.com/can1357/oh-my-pi/tree/39c95e5e29b1c8b082059f57421ce445c3dffdd4)

## Decision summary

Dynamic, temporary Agents are the primary delegation primitive.

- The main Agent decomposes a task into one or more concrete temporary tasks in
  a single tool call.
- Independent tasks run concurrently; explicit dependencies form a DAG.
- Every task may optionally use a reusable Specialist persona, but a Specialist
  is not required to create a sub-Agent.
- Capabilities, model selection, and execution backend are separate concerns.
  A code-capable Agent does not imply Codex, ACP, or any particular model.
- Wisp's native Agent runtime is the default executor. ACP is an optional
  executor selected from configured ACP profiles.
- Results are schema-validated and returned to the parent tool call so the main
  Agent can continue the same turn and synthesize the user-facing answer.
- Persistence, approval, cancellation, budgets, attempts, child conversations,
  and the Agents panel remain as the control and audit plane.

The current implementation is a useful workflow runner, but its four compiled
templates, second planner-model call, mandatory final Reviewer, Codex-specific
ACP routing, and lack of parent result delivery are the wrong product
abstraction for this goal.

## Goals

1. Let the main Agent create an ad-hoc team sized and shaped for the current
   task, including a single child, parallel fan-out, sequential work, or any
   acyclic fan-out/fan-in graph.
2. Make the common path one `delegate_tasks` call, not a separate planning
   conversation followed by a manually operated workflow panel.
3. Support both generic temporary workers and reusable Specialist personas.
4. Run useful read, edit, code, analysis, literature, visualization, and review
   work through Wisp's native runtime without requiring ACP.
5. Keep ACP vendor-neutral and optional. Any configured ACP Agent may be used
   when its profile and permission boundary satisfy the task.
6. Return compact structured results to the parent automatically while keeping
   full output, evidence, artifacts, usage, and child transcripts persisted.
7. Preserve explicit approval for file writes, command execution, network use,
   external side effects, and elevated budgets.
8. Keep the design cross-platform and aligned with the research-workbench
   nouns: compute should create `Run` records and outputs should become
   `Artifact`, `DataAsset`, `Paper`, or `Decision` references where applicable.

## Non-goals for the first delivery

- Inventing a proprietary generic HTTP Agent protocol. Direct HTTP/local model
  profiles already run inside the native executor; external autonomous Agents
  should use ACP or a future concrete protocol.
- Giving a model arbitrary tool names, paths, process commands, backend launch
  commands, credentials, or raw permission objects.
- Running parallel writers in the same checkout before isolation exists.
  Writable tasks are initially serialized while read-only work can remain
  parallel.
- Treating delegated Agents as a replacement for the Run Manager. Long-running
  scientific compute must be launched and tracked as a structured `Run`.
- Adding peer chat, a YAML swarm language, process-global Agent state, or a
  large settings matrix before a real workflow requires it.

These are staged boundaries, not architectural dead ends. Executor profiles,
capability definitions, persisted DAGs, and bounded delegation depth provide
the extension points without implementing speculative transports or schedulers.

## What to borrow from oh-my-pi

The useful ideas are documented in its
[`task` tool](https://github.com/can1357/oh-my-pi/blob/39c95e5e29b1c8b082059f57421ce445c3dffdd4/docs/tools/task.md),
[`TaskItem` contract](https://github.com/can1357/oh-my-pi/blob/39c95e5e29b1c8b082059f57421ce445c3dffdd4/packages/coding-agent/src/task/types.ts),
and
[`executor`](https://github.com/can1357/oh-my-pi/blob/39c95e5e29b1c8b082059f57421ce445c3dffdd4/packages/coding-agent/src/task/executor.ts).

| Borrow | Wisp adaptation |
| --- | --- |
| One thin batch tool creates temporary sub-Agents | `delegate_tasks` accepts shared context and a list of self-contained tasks |
| Children start with blank conversation history | Pass a bounded shared context, project rules, explicit inputs, and direct dependency results |
| Session-scoped concurrency semaphore | Reuse Wisp's DAG scheduler and add capability-specific lanes |
| Optional reusable Agent definitions | Reuse Wisp Specialists as optional personas, not mandatory templates |
| Structured terminal yield and schema validation | Keep a standard Wisp result envelope and allow a bounded per-task output schema |
| Background jobs auto-deliver completion | Inline is the default; persisted background completion can inject a result and resume later |
| Durable output IDs and child transcripts | Reuse workflow attempts, child frames, Artifact IDs, and a read-only result lookup |
| Optional isolated workspaces | Add cross-platform git-worktree/patch isolation only after the native mutation path works |
| Bounded recursive spawning | Add a root-wide depth/task/budget ceiling after one-level delegation is stable |

We should not copy oh-my-pi's coding-only assumptions, unrestricted general
worker, process-global registry, IRC coordination layer, Linux-specific
isolation backends, or separate YAML swarm subsystem. Wisp already has SQLite
workflow state, a dependency-aware executor, approval policies, and scientific
project resources.

## Product model: orthogonal dimensions

Every delegated task is resolved along independent dimensions. Supporting all
meaningful combinations comes from composing these dimensions, not multiplying
hard-coded Agent classes.

| Dimension | Examples | Authority |
| --- | --- | --- |
| Origin/persona | temporary generic worker, built-in Reviewer, custom Specialist | Main Agent proposes; runtime resolves and snapshots |
| Capabilities | project read, project write, code/run, literature, network research, visualization, review | Main Agent requests IDs; runtime grants from a controlled registry |
| Model | inherit current model, Specialist-bound profile, user override | Runtime/user policy; never inferred from executor brand |
| Executor | native Wisp, configured ACP profile, future registered executor | Runtime resolver and user override |
| Topology | one task, parallel batch, sequence, arbitrary DAG | Main Agent supplies dependency IDs; runtime validates |
| Completion | inline wait, persisted background | Conversation policy/user action |
| Workspace | shared read-only, serialized mutation, isolated checkout | Runtime safety policy |
| Output | standard result envelope, task-specific JSON schema | Runtime validates and persists |

Three distinctions are invariants:

1. **Specialist is a persona, not an executor.** Instructions, skills,
   connectors, and a preferred model do not grant write/run/network access.
2. **Capability is intent, not a raw tool list.** Only the host maps capability
   IDs to tools, paths, approval class, and budget ceilings.
3. **Executor is transport/runtime, not a model identity.** Native execution
   can use any configured local or HTTP model; ACP can launch any compatible
   ACP Agent profile.

## Main-Agent tool contract

The model-facing tool is `delegate_tasks`. Its schema is generated from the
currently available capability IDs and spawnable Specialist summaries.

```json
{
  "goal": "Compare two analysis strategies and recommend one",
  "context": "Shared constraints, relevant project state, and expected final outcome",
  "tasks": [
    {
      "id": "inspect-data",
      "instruction": "Inspect the registered input data and report shape, quality issues, and assumptions.",
      "depends_on": [],
      "capabilities": ["project_read"],
      "output_schema": {
        "type": "object",
        "properties": { "summary": { "type": "string" } },
        "required": ["summary"]
      }
    },
    {
      "id": "literature",
      "instruction": "Find and compare source-supported guidance for the two methods.",
      "depends_on": [],
      "capabilities": ["literature_search"]
    },
    {
      "id": "evaluate",
      "instruction": "Evaluate both methods using the inspected data and return reproducible evidence.",
      "depends_on": ["inspect-data", "literature"],
      "capabilities": ["project_read", "code_run"]
    },
    {
      "id": "review",
      "instruction": "Independently check the evidence and list material weaknesses.",
      "depends_on": ["evaluate"],
      "capabilities": ["review"],
      "specialist_id": "reviewer"
    }
  ]
}
```

### Model-visible task fields

- `id`: required, unique within the batch, stable for dependencies and results.
- `instruction`: required and self-contained; includes the concrete outcome and
  acceptance criteria for this child.
- `depends_on`: zero or more task IDs from the same call.
- `capabilities`: one or more IDs advertised in the dynamic tool description.
- `specialist_id`: optional reusable persona. Omission creates a generic
  temporary Agent.
- `output_schema`: optional bounded JSON Schema for `data` inside the standard
  result envelope.
- `isolated`: optional request for an isolated writable workspace; the runtime
  may strengthen this or reject it, but never weaken required isolation.

The model does not submit `tools`, `permissions`, `backend`, executable/args,
credentials, absolute paths, token/cost ceilings, or concrete model profile
IDs. Those are host/user decisions.

### Host-side overrides

The persisted draft and manual editor may carry user-controlled overrides not
shown to the model:

- concrete model profile ID;
- concrete executor profile ID;
- smaller task/batch budget;
- inline/background completion;
- isolation/merge choice;
- maximum parallelism.

Overrides may reduce or select within resolved authority; they cannot exceed
project and application policy ceilings.

## Resolved task snapshot

Before approval or execution, a pure policy resolver converts every proposal
into an immutable `ResolvedAgentTask` snapshot:

```text
ResolvedAgentTask
  id, instruction, dependencies
  origin: temporary | specialist snapshot
  capability ids and registry revisions
  final prompt and bounded context policy
  exact tool/path/network/write permissions
  resolved model profile
  resolved executor profile
  timeout, token/tool/cost budgets
  output contract
  workspace/isolation policy
  approval reasons
```

The snapshot stores the Specialist instructions and relevant allowlists used at
approval time. Editing or deleting the Specialist later cannot mutate an
approved workflow. Credentials remain in the keyring/profile stores and are
referenced only by non-secret IDs.

Execution revalidates the snapshot against current capability ceilings and
executor availability. A persisted or model-produced object is never treated
as authorization merely because it deserializes.

## Capability registry

`CapabilityRegistry` replaces fixed workflow templates as the safety boundary.
A capability definition contains:

```text
id, display name, description
risk class
native tool names
required executor features
path and context ceilings
default and maximum budgets
approval explanation
```

Initial built-ins should reuse existing tools and project configuration:

| Capability | Native behavior | Risk |
| --- | --- | --- |
| `reasoning` | model only, no project tools | read-only |
| `project_read` | read/search/grep within project scope | read-only |
| `project_write` | project read plus write/edit | write |
| `code_run` | structured Run/ExecutionContext tools; bounded shell only where already permitted | execute |
| `literature_search` | configured literature skills/connectors | network/read, depending on connector |
| `external_research` | approved web/MCP connectors | network |
| `visualization` | project read/write plus bounded runtime tools | write/execute |
| `review` | read-only project evidence plus host-captured diff/provenance | read-only |
| `image_inspection` | configured vision path | external model/read |

Capabilities may compose. The resolver takes the union of required features
and the most restrictive applicable ceilings. Unknown/disabled IDs fail
closed. A Specialist can narrow available skills/connectors, but cannot widen
the task's capability grant.

## Executor registry and selection

`AgentDelegator` remains the execution interface. A small executor registry
adds discovery and capability matching around it.

### Native Wisp executor

- Built in and selected by default.
- Uses the ordinary configured Wisp model path, including local models and
  direct HTTP providers.
- Constructs a child tool registry from the resolved capability grant.
- Uses a project-scoped delegated `ToolEnv` for approval, cancellation, path
  validation, and Run Manager access.
- Creates a child frame and persists the same provenance/usage fields as today.
- Does not expose `delegate_tasks` to a child until bounded nested delegation is
  explicitly enabled.

### ACP executor

- Selected by a configured ACP profile ID, never by command-string matching.
- Code and visualization work can use Native or any eligible executor.
- Uses ACP initialization/session capabilities, Wisp's permission response
  boundary, and the resolved capability grant.
- Receives only the Wisp MCP bridge tools allowed by the task. A task with no
  bridge capability gets no bridge.
- Preserves ACP session provenance and optional reuse behavior.

### Other executors

- A direct HTTP/local LLM is not a separate executor; it is a model used by the
  native executor.
- A future remote ACP transport or concrete external service registers another
  executor profile with advertised features and implements `AgentDelegator`.
- The legacy `AgentBackend::Http` and `Custom` values must not pretend to work.
  They remain deserialization-compatible until either a concrete executor is
  registered or the legacy format is retired.

Automatic selection prefers native, then an explicitly configured eligible
executor. It never silently falls from an isolated/sandboxed request to a less
protected executor. The Agents panel shows both the requested preference and
the resolved executor.

## Planning and scheduling

There is no second planner-model call. The main Agent already has the task,
conversation context, and tool schema; it submits the decomposition directly.

The existing dependency-aware scheduler remains the core:

1. Validate non-empty unique IDs, known dependencies, acyclicity, task count,
   concurrency, and root-wide budgets.
2. Persist the proposal and resolved immutable plan before execution.
3. Start all dependency-ready tasks subject to the session semaphore and
   capability lanes.
4. Attach each direct dependency's structured result to the dependent child.
5. Continue independent branches after an unrelated branch fails.
6. Mark descendants of a failed/cancelled task blocked.
7. Return all terminal and partial results to the parent.

Initial safety limits:

- at most 8 tasks in one batch;
- default maximum parallelism 2, configurable later up to a hard ceiling of 4;
- one shared mutation lane until isolation is implemented;
- one-level delegation only;
- a batch-level token/tool/cost/time ceiling in addition to per-task ceilings.

These limits prevent accidental Agent explosions while still supporting the
intended parallel behavior.

## Context policy

Children start without the parent transcript. Each receives only:

- the task instruction;
- the bounded batch `context`;
- applicable project instructions such as `AGENTS.md`;
- explicitly referenced `Artifact`, `DataAsset`, `Paper`, or file inputs;
- the structured results of direct dependencies;
- its resolved capability and completion contract.

Large data is passed by durable reference, not copied into prompts. A child
does not automatically inherit every MCP connection, skill, memory, or secret.

## Result and parent feedback contract

Every task returns a standard envelope. `data` is validated against the
optional task-specific schema.

```json
{
  "id": "evaluate",
  "status": "succeeded",
  "summary": "Short parent-ready conclusion",
  "data": {},
  "artifacts": [],
  "evidence": [],
  "tests": [],
  "risks": [],
  "usage": {},
  "child_frame_id": "...",
  "error": null
}
```

The batch tool result contains workflow ID, overall state, results in input
order, and blocked/failure information. It is a successful tool response even
when one independent task failed, because partial evidence is useful to the
parent; schema/authorization/runtime failure of the delegation mechanism itself
is a failed tool response.

For inline execution, this compact JSON is returned directly to the parent
Agent, which continues its tool loop and synthesizes the answer in the same
turn. Full output remains in workflow attempts and child frames. A read-only
result lookup can retrieve a specific full result without reinlining every
child transcript.

For background execution, the initial result is a durable handle. Completion
is appended exactly once to the owning conversation as a structured internal
result; an optional auto-resume policy lets the main Agent synthesize when the
conversation is idle. The Agents panel is observability and control, not the
only way to consume results.

## Approval policy

Delegation remains opt-in per conversation. Once enabled, the conversation has
one of two simple approval policies:

- `review_all`: show the resolved batch before any child starts;
- `auto_safe`: automatically run native read-only work, but review any batch
  containing write, execute, network, external-side-effect, isolation merge,
  ACP/external executor, or elevated-budget work.

The runtime uses the existing approval channel so an inline tool call may pause,
show the exact tasks/capabilities/executors, then continue after approval. A
denial with feedback returns to the main Agent so it can revise the batch.

Approved task specs are immutable. Retries reuse the approved snapshot unless
the user explicitly creates a revised draft. Disabling delegation blocks new
spawns and retries but does not hide history or implicitly cancel running work.

## Specialists

The current `Specialist` record already supplies the useful persona fields:
instructions, model binding, skills, and connectors. Delegation adds only an
optional reference from a task and, if needed, a user-controlled executor
preference. It does not create another mandatory Agent-template registry.

- No `specialist_id`: create a generic temporary worker with the resolved task
  prompt and capabilities.
- With `specialist_id`: snapshot that Specialist's instructions and apply its
  model/skill/connector restrictions.
- Built-in Reviewer is an optional persona, not an automatically appended step.
- Code execution is a capability of any temporary or Specialist-backed task;
  it does not imply a permanent role or ACP executor.

Later, shareable project Agent definitions may be discovered from a project
directory, but only when a concrete sharing/import need exists. The current
SQLite Specialist CRUD is enough for the first complete flow.

## Workspace mutation and isolation

Read-only tasks may run together in the project workspace. Before isolated
write support lands, all tasks with `project_write`, `code_run`, or
`visualization` enter one mutation lane and therefore cannot concurrently edit
the same checkout.

The next isolation layer is deliberately small and cross-platform:

- detect a Git repository and an installed Git executable;
- create one temporary worktree/branch per writable child using argument arrays,
  never shell command strings;
- capture a patch and changed-file manifest;
- validate conflicts before applying/cherry-picking;
- always clean up on success, failure, or cancellation;
- fall back to serialized shared-workspace execution for non-Git projects.

Do not add overlayfs/APFS/ZFS/ProjFS implementations until measured performance
requires them.

## Nested delegation

Nested delegation is an extension of the same protocol, not a second system.
It is disabled for the first release. When enabled:

- a root workflow has maximum depth, total child count, concurrency, token,
  tool-call, cost, and wall-clock budgets;
- children receive `delegate_tasks` only below the depth limit and only when
  their resolved policy allows it;
- child IDs and attempts record root workflow and parent attempt lineage;
- nested results roll up through the same structured result envelope;
- cancellation and budget exhaustion propagate from root to descendants.

Default depth remains 1; the first opt-in maximum is 2. Peer messaging is not
required for nested delegation. Explicit dependencies and persisted artifacts
are the primary coordination mechanism.

## Persistence and unsupported prior records

Reuse `agent_workflows`, `agent_workflow_steps`, and
`agent_workflow_attempts`. They already provide versioned drafts, immutable
approved steps, attempts, statuses, cancellation, usage, evidence, child frame
IDs, and ACP session IDs.

- Require `schema_version: 2` in serialized plan JSON.
- Dynamic plans store origin, capability IDs, resolved
  executor/model references, isolation, and the immutable policy snapshot in
  `plan_json`/`spec_json`.
- Avoid a SQL migration for fields already safely represented in those JSON
  snapshots.
- Do not deserialize, display, revise, approve, retry, or execute earlier plan
  formats. Their rows remain unchanged in SQLite and are intentionally inert.
- Never reinterpret an earlier executor-specific record as a dynamic Native
  plan, and do not add a migration that guesses new capabilities or authority.

A later migration is justified only for fields that need indexed queries or
atomic delivery, such as background-result delivery time or nested lineage.

## UI behavior

The Agents panel becomes an activity and approval surface scoped to the active
conversation:

- running/recent batches owned by the active conversation, including nested
  workflows;
- task rows with dependency, capability, Specialist, model, executor, status,
  duration, usage, and current tool summaries;
- dynamic draft editing for instruction, dependencies, capabilities, persona,
  model/executor override, and isolation;
- approve, deny with feedback, cancel, retry, inspect result, and take over a
  child conversation;
- clear indication that results also return to the parent conversation.

Temporary child conversations remain persisted for audit and takeover, but
are linked beneath the dispatching conversation and are excluded from the
top-level session history, recent-session views, and session search.

The predefined role buttons disappear.
Manual creation remains possible through an “Add task” editor using the same
dynamic contract and resolver as automatic delegation.

## Security invariants

1. Model output is a proposal, never an authorization grant.
2. Unknown capability, Specialist, model, executor, dependency, path, or output
   schema fails closed.
3. Capabilities resolve to an allowlist; no child receives all tools by default.
4. Specialist settings can only narrow task authority.
5. Secrets stay in keyring/profile storage and never enter plan JSON or prompts.
6. Approved snapshots are immutable and revalidated before every attempt.
7. Write, execute, network, external side effect, and elevated budgets remain
   visible approval boundaries.
8. Cancellation is root-aware and does not imply rollback; created files/runs
   and partial artifacts remain visible.
9. Long-running compute is a `Run`, not an unbounded shell tool call.
10. Windows and macOS paths/processes are explicit; no Unix-only isolation is
    assumed.

## Success criteria

The redesign is complete when all of these are true:

- A main Agent can create two independent temporary read-only Agents in one
  call, both actually overlap in execution, and their results return to the
  same parent turn.
- A dependent third task receives the first two structured results and runs
  only after both succeed.
- A generic native task can read, edit, and execute within approved project
  capability boundaries without any ACP profile installed.
- Selecting a configured non-Codex ACP Agent works without vendor-name checks.
- Any existing custom Specialist can be selected for one temporary task; tasks
  without a Specialist also work.
- An elevated batch shows exact approval reasons; denial feedback reaches the
  parent and no child starts.
- Partial failures, cancellation, retry, usage, evidence, artifacts, child
  frames, and full results are persisted and visible.
- Earlier plan records remain unchanged in storage but are neither exposed nor
  executable.
- Automated tests require no API key, network, ACP executable, SSH host, WSL,
  scheduler, or GPU.
