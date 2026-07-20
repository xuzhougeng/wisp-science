# Dynamic Temporary Agent Delegation — Implementation Plan

**Goal:** Replace fixed-template workflow creation with automatically generated,
temporary, parallel sub-Agents whose capabilities, model, executor, persona,
dependencies, and completion mode are independent and policy-resolved.

**Design:**
[`docs/superpowers/specs/2026-07-20-dynamic-agent-delegation-design.md`](../specs/2026-07-20-dynamic-agent-delegation-design.md)

**Delivery rule:** This is a sequence of small PRs. Do not combine the entire
roadmap into one branch. PRs 1-12 leave legacy delegation readable; PR 13 is
the intentional breaking retirement point. Every intermediate PR must leave
the workspace testable.

## Outcomes by milestone

| Milestone | Outcome |
| --- | --- |
| A — correct core | The main Agent can create a dynamic native batch, independent tasks overlap, dependency results flow forward, and all results return to the parent turn. |
| B — complete daily product | Dynamic approval/editor UI, native scientific capabilities, optional generic ACP, and existing Specialists all work in the same protocol. |
| C — advanced execution | Background delivery, isolated parallel writers, bounded nesting, and legacy fixed-plan retirement. |

The first useful release is Milestone A. It already removes Codex/ACP as a
requirement and implements the user's original temporary parallel-Agent model.
Milestones B and C fill out every supported combination without delaying the
core correction.

## Global constraints

- Reuse `DelegationExecutor`, workflow/step/attempt persistence, cancellation,
  child frames, approval transitions, and usage/evidence storage.
- Do not create a second scheduler, workflow table family, or planner-model
  call.
- Model output may request only controlled capability/Specialist IDs. It never
  supplies raw tools, permissions, credentials, executable commands, model
  secrets, or backend configuration.
- Native Wisp is the default executor. ACP is optional and vendor-neutral.
- PR 13 removes all prior-plan deserialization and behavior; those records stay
  untouched in SQLite but receive no UI, command, retry, or execution path.
- Initially serialize writable tasks in one mutation lane. Do not claim safe
  parallel edits before isolation exists.
- Long-running scientific work must go through Run Manager APIs rather than a
  larger shell timeout.
- Add no real API, ACP process, SSH, WSL, GPU, scheduler, or network dependency
  to tests.
- Keep Windows and macOS path/process behavior explicit.
- Add or update user-facing English and Chinese strings together.
- Update `docs/agent-delegation.md` whenever shipped behavior changes.

Every implementation PR ends with the narrow relevant tests, followed by:

```bash
cargo fmt --all -- --check
cargo test --workspace
cd ui && cargo check --target wasm32-unknown-unknown
cd ../ui-tests && npm ci && npx playwright test
```

Run the MCP smoke example too when the delegation tool exposed by the Wisp MCP
bridge changes:

```bash
cargo run -p wisp-mcp --example smoke
```

## Existing code to retain or change

| Current area | Decision |
| --- | --- |
| `crates/wisp-core/src/execution.rs` | Retain the DAG scheduler; extend result semantics and policy validation only. |
| `crates/wisp-store` Agent workflow tables | Retain; use versioned `plan_json`/`spec_json` before adding columns. |
| `StoreDelegationObserver` | Retain as the attempt/provenance persistence boundary. |
| Child conversation creation and Take over | Retain for inspection and follow-up. |
| `AgentTemplateRegistry::builtins()` | Legacy v1 only, then retire. It is not the v2 extensibility mechanism. |
| `DelegationPlanner` and planner-model call | Stop using for v2; delete after UI migration. |
| `LocalDelegator` | Evolve into a capability-filtered native executor. |
| `AcpDelegator` | Keep, remove Codex detection, route by executor profile/capabilities. |
| `propose_delegation` | Keep temporarily for v1 compatibility; replace main prompt usage with `delegate_tasks`. |
| Specialists | Reuse as optional personas; do not duplicate CRUD or require one per task. |
| Agents panel | Convert from fixed-team builder to activity, dynamic draft, and approval surface. |

New modules are justified only at real dependency/test boundaries. In
particular, extracting a pure capability resolver into `wisp-core` and
separating native/ACP executor code are valid boundaries; splitting a file only
because it is long is not.

---

## PR 0 — Commit the design and migration plan

**Files**

- Add:
  `docs/superpowers/specs/2026-07-20-dynamic-agent-delegation-design.md`
- Add: `docs/superpowers/plans/2026-07-20-dynamic-agent-delegation.md`

**Work**

- Record the product decisions, v1 compatibility policy, protocol shape,
  security invariants, and PR sequence.
- Reference the specific oh-my-pi implementation studied, while documenting
  the Wisp-specific adaptations and deliberate omissions.

**Acceptance**

- A contributor can answer where task identity, capability, model, executor,
  approval, scheduling, persistence, and result synthesis are owned.
- No runtime behavior changes.

---

## PR 1 — Add a versioned dynamic plan contract

**User-visible change:** None. This is the backward-compatible data boundary.

**Primary files**

- Modify: `crates/wisp-core/src/orchestration.rs`
- Modify: `crates/wisp-core/src/delegation.rs`
- Modify: `crates/wisp-core/src/lib.rs`
- Add fixtures/tests beside the affected core modules.

**Contract changes**

1. Add `schema_version` to `DelegationPlan`, with serde default `1`.
2. Add v2 task metadata with serde defaults so v1 JSON still parses:
   - `AgentOrigin`: temporary or a snapshotted Specialist;
   - requested/resolved capability IDs;
   - `ExecutorRef`: native or configured ACP/external profile reference;
   - workspace policy: shared read-only, serialized mutation, or isolated;
   - task-specific output schema provenance;
   - approval reasons.
3. Keep legacy `template_id`, backend, prompt, permissions, context, and budget
   fields readable. A v2 temporary task uses an explicit origin, not a fake
   user-facing fixed template.
4. Separate structural plan validation from authorization validation:
   - structural: IDs, task count, dependencies, cycles, concurrency, required
     strings, output-schema size/shape;
   - authorization: supplied by the capability resolver in PR 2.
5. Do not remove `AgentTemplateRegistry` or change v1 execution in this PR.

**Tests**

- Deserialize representative v1 plan/spec JSON and round-trip it unchanged in
  meaning.
- Validate a v2 single task, parallel tasks, a fan-in task, and input-order
  preservation.
- Reject duplicate IDs, missing dependencies, self-dependencies, cycles, more
  than 8 tasks, invalid concurrency, and oversized/invalid output schemas.
- Assert v1 validation still rejects a tampered fixed template.

**Commit boundary**

Only contract and pure validation changes. No Tauri/UI behavior.

---

## PR 2 — Introduce the capability policy resolver

**User-visible change:** None. This creates the new authorization boundary.

**Primary files**

- Add: `crates/wisp-core/src/delegation_policy.rs`
- Modify: `crates/wisp-core/src/lib.rs`
- Modify: `crates/wisp-core/src/delegation.rs`
- Modify: `crates/wisp-core/src/orchestration.rs`

**Core types**

```text
CapabilityDefinition
CapabilityRegistry
DelegatedTaskProposal
DelegationHostPolicy
ResolvedAgentTask
ResolvedDelegationPlan
ResolutionError
```

**Work**

1. Define built-in capabilities: `reasoning`, `project_read`,
   `project_write`, `code_run`, `literature_search`, `external_research`,
   `visualization`, `review`, and `image_inspection`.
2. A definition owns risk class, native tool IDs, required executor features,
   context/path ceilings, budget ceilings, and approval text.
3. Resolve a proposal by:
   - validating capability IDs;
   - composing capability requirements;
   - intersecting project/session/Specialist ceilings;
   - selecting an eligible executor/model reference from host policy;
   - computing exact permissions, budgets, isolation, and confirmation;
   - producing an immutable v2 spec snapshot.
4. Replace the hidden `AgentTemplateRegistry::builtins()` lookup inside the
   delegation request validation boundary with an explicit v1-or-v2 validator.
   The caller cannot accidentally validate a v2 request against compiled
   templates or bypass capability authorization.
5. Update `DelegationExecutor` to use the legacy template validator only for a
   v1 plan and the resolved-policy validator for a v2 plan; its default
   constructor must not silently reapply built-in templates to v2 steps.
6. Keep actual capability availability separate from definition. A capability
   such as literature search is not advertised when its required connector is
   unavailable.

**Tests**

- Exact permission/tool grants for each built-in capability.
- Composed capabilities take the highest risk and most restrictive ceilings.
- Unknown/disabled capability fails closed.
- Specialist restrictions can narrow but never widen a grant.
- Model/executor overrides select only configured eligible profiles.
- Native read-only batches do not require confirmation under `auto_safe`.
- Write, execute, network, external, ACP, isolation merge, and elevated budget
  each produce an explicit confirmation reason.
- Tampering with permissions, executor, prompt snapshot, or budget after
  resolution is rejected before delegation.

**Commit boundary**

Pure policy logic and tests; legacy runtime remains the only caller.

---

## PR 3 — Make native delegated Agents genuinely capable

**User-visible change:** A v2 Agent can use Wisp's configured model and scoped
native project tools without any ACP profile.

**Primary files**

- Modify: `crates/wisp-tools/src/lib.rs`
- Modify: `src-tauri/src/delegation_runtime.rs`
- Modify: `src-tauri/src/models.rs` only if profile resolution needs a reusable
  helper.
- Add a separate native-executor module only if doing so creates the needed
  fake-provider/tool-policy test boundary.

**Work**

1. Add a minimal `Registry` constructor/filter that creates only an approved
   set of existing tools. Do not build a second tool registry abstraction.
2. Replace `run_local_agent`'s second hand-written completion/tool loop with the
   existing `wisp_core::agent_loop`, an independent child `ContextManager`, the
   normal Wisp tool registry, and a delegated `ToolEnv`. Do not use the main
   session's `.wisp/session.json`; SQLite child-frame messages remain the
   durable child transcript. The delegated environment enforces:
   - project path scope;
   - exact resolved tool allowlist;
   - approval mode;
   - cancellation polling;
   - no secret propagation;
   - Run Manager access only through the resolved capability.
3. Rename the behavioral concept from Local to Native in new v2 surfaces. Keep
   the legacy serialized `local` value as an alias while v1 plans exist.
4. Resolve a v2 model profile independently from the executor. Default to the
   active model; a Specialist/user override may select another existing model
   profile.
5. Implement the first complete native grants:
   - reasoning: no tools;
   - project read: read/search/grep;
   - project write: read/search/grep/write/edit;
   - review: read-only plus host-captured evidence;
   - code/run: the existing bounded execution/Run surface available in the
     app, never an increased shell timeout.
6. Continue persisting child frames, messages, result envelopes, evidence,
   artifacts, usage, and attempts.
7. Make native cancellation targeted and terminal; `TauriDelegator::cancel`
   must not be ACP-only for v2.

**Tests**

- Fake provider requests an allowed read tool and returns a valid result.
- Fake provider requests a disallowed write/run/network tool and is denied.
- Temporary-directory read/write stays inside the project; traversal and
  absolute out-of-scope paths fail.
- Cancellation stops the native loop and records a cancelled attempt.
- Budget/tool-call exhaustion fails with preserved usage/provenance.
- A write-capable native task works with no ACP profiles configured.
- Reviewer stays read-only and receives independently captured host evidence.

**Manual smoke**

- With only an ordinary Wisp model configured, run one read-only and one
  approved edit task; inspect child frames and persisted attempt details.

---

## PR 4 — Add inline `delegate_tasks` with automatic parent synthesis

**User-visible change:** The main Agent can fan out temporary tasks and receive
their results in the same turn.

**Primary files**

- Modify or replace the v2 portion of: `src-tauri/src/delegation_tool.rs`
- Modify: `src-tauri/src/delegation_runtime.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/mcp_bridge.rs`
- Modify: `crates/wisp-core/src/execution.rs` only for result semantics needed
  by the tool.
- Update: `docs/agent-delegation.md`

**Work**

1. Register `delegate_tasks` only when delegation is enabled for the owning
   conversation.
2. Expose the batch schema from the design: goal, shared context, tasks, IDs,
   dependencies, capability IDs, optional Specialist, optional output schema,
   and isolation request.
3. Update the delegation prompt section: the main Agent itself decomposes the
   work and must synthesize the returned results. Do not call
   `model_selected_templates`.
4. On execution:
   - parse and structurally validate;
   - resolve capabilities/persona/model/executor;
   - persist v2 workflow and immutable steps;
   - use the existing approval channel when policy requires it;
   - approve and execute inline;
   - return compact ordered JSON results to the tool loop.
5. Independent branches keep running after an unrelated failure. Descendants
   of failed/cancelled tasks become blocked. Return partial results as useful
   data, not as an opaque tool failure.
6. Cap inline result text. Persist full results and add a read-only lookup tool
   or command for one workflow/task result when the parent needs more detail.
7. Keep `propose_delegation` registered for the legacy UI during migration, but
   stop recommending it in the main Agent prompt.
8. Expose the equivalent Wisp MCP bridge tool and capability metadata with the
   same conversation opt-in check.

**Tests**

- Agent-loop integration with a fake parent model:
  1. parent calls `delegate_tasks`;
  2. two fake children complete;
  3. tool result is appended;
  4. parent produces a final synthesis in the same turn.
- Two independent fake tasks overlap while respecting `max_parallel=2`.
- Fan-in task starts only after both dependencies and receives both structured
  results.
- Denied approval starts zero children and returns feedback to the parent.
- One failed branch yields partial results and blocks only its descendants.
- Output schema valid/invalid cases.
- Delegation-off session cannot see or invoke the tool.
- MCP smoke verifies listing and dispatch when enabled.

**Manual smoke**

- Ask the main Agent to compare two files using parallel temporary Agents and
  verify that the final chat response, not only the Agents panel, includes the
  combined result.

**Milestone A gate**

Do not begin broad UI or ACP changes until this flow works end to end with a
native model and no ACP profile.

---

## PR 5 — Add v2 workflow commands and approval payloads

**User-visible change:** Dynamic drafts can be inspected, edited, approved,
retried, and cancelled through stable backend commands.

**Primary files**

- Modify: `src-tauri/src/delegation_runtime.rs`
- Modify: `crates/wisp-store/src/agent_workflows.rs` only if an atomic behavior
  is missing; prefer existing `plan_json`/`spec_json`.
- Modify: `src-tauri/src/lib.rs` command registration.
- Modify DTOs in `ui/src/dto.rs` only after backend payloads are stable.

**Work**

1. Add v2 DTOs for proposal, resolved task summary, approval reason, executor
   summary, result summary, and version conflict.
2. Let a draft edit instruction, dependency IDs, capability IDs, Specialist,
   concrete model/executor override, isolation, and a smaller budget.
3. Every revision reruns structural and authorization resolution and increments
   the existing optimistic version. Approved steps remain immutable.
4. Use one simple conversation approval policy:
   - `review_all`;
   - `auto_safe`.
5. Map old workflow modes for display only:
   - manual/assisted -> review all;
   - automatic -> auto safe.
6. Ensure approve/run/retry execute the stored resolved v2 snapshot rather than
   planning again.

**Tests**

- Optimistic revision conflict.
- Removing a dependency updates validation; introducing a cycle fails.
- Capability/model/executor edits recalculate approval reasons.
- Specialist edit after approval does not change the snapshot.
- Retry uses the approved snapshot.
- Imported/interrupted v2 workflows recover to an explicit terminal state.

---

## PR 6 — Replace the fixed-team UI with dynamic activity and drafts

**User-visible change:** The Agents panel represents what is actually running,
not a compiled list of four Agent buttons.

**Primary files**

- Add: `ui/src/agent_workflows.rs`
- Modify: `ui/src/main.rs`
- Modify: `ui/src/dto.rs`
- Modify: `ui/src/i18n.rs`
- Modify relevant styles in `ui/src/styles.css` or the existing scoped files.
- Modify: `ui-tests/tests/ui.spec.ts`
- Update: `docs/agent-delegation.md`

**Work**

1. Remove the fixed Biology/Code/Reviewer/Visualization add buttons for v2.
2. Show batches grouped by conversation with task rows containing:
   dependency chips, capabilities, Specialist, model, executor, status,
   duration, usage, current activity, and result availability.
3. Add a dynamic draft editor and an advanced manual “Add task” action. Manual
   tasks use exactly the same proposal/resolver path as main-Agent tasks.
4. Show all approval reasons and resolved authority before approval.
5. Preserve cancel, retry, discard, inspect result, and Take over.
6. Make the primary text clear: inline results return to the parent chat; the
   panel is for control and audit.
7. Keep keyboard navigation, focus visibility, semantic labels, and English /
   Chinese parity.

**Playwright cases**

- Main-created parallel v2 batch renders two running rows and a dependent
  pending row.
- Risky draft shows reasons, edits, approves, and transitions to running.
- Manual arbitrary task creation does not require a pre-existing template.
- Partial failure and blocked descendant render distinctly.
- Result inspection and Take over route to the correct child frame.
- Delegation disabled state preserves history but blocks new work.
- Legacy v1 workflow still renders with a Legacy badge and existing controls.

**Manual smoke**

- Repeat the flow shown in the original screenshots and verify no fixed-team
  selection is needed; the main Agent creates the temporary tasks itself.

---

## PR 7 — Make executor selection generic and ACP optional

**User-visible change:** Any configured eligible ACP Agent can be selected, but
native remains the default and fully functional.

**Primary files**

- Modify: `src-tauri/src/delegation_runtime.rs`
- Modify: `src-tauri/src/acp.rs` as needed for profile capability summaries.
- Modify core executor reference types and UI DTOs.
- Add/extract an executor registry module only to isolate profile resolution
  and fake-executor tests.

**Work**

1. Register the built-in Native executor and each configured ACP profile as an
   `ExecutorProfileSummary` with stable ID, kind, availability, and supported
   features.
2. Automatic resolution prefers Native when it satisfies the task. A user may
   override with an eligible configured ACP profile.
3. Delete `is_codex_profile`, the template-ID check for code/visualization, and
   `controlled_codex_launch_profile` naming/behavior. Launch the selected ACP
   profile through the normal ACP client.
4. Enforce resolved task permissions on ACP permission requests regardless of
   vendor.
5. Pass a filtered Wisp MCP bridge only when the resolved task grants matching
   capabilities. Do not expose the whole bridge by default.
6. Never fall back from requested isolation or permission guarantees to a less
   capable executor.
7. Treat direct HTTP/local models as Native model choices, not Agent executors.
   Leave legacy `Http`/`Custom` serialization readable but unavailable unless a
   concrete executor is registered.

**Tests**

- Auto selects Native with no ACP profile.
- Explicit eligible ACP profile is selected.
- A fake non-Codex ACP profile executes a code-capable request.
- Missing/ineligible ACP profile fails before child creation.
- ACP permission requests outside project/capability scope are denied.
- Filtered bridge contains only granted tools.
- Cancellation, session reuse, fingerprint mismatch, usage, and provenance
  remain covered without a real ACP executable.

**Manual smoke**

- Configure two different ACP Agent profiles; select each for a harmless
  read-only task and verify the panel reports the actual profile.

---

## PR 8 — Allow existing Specialists to back temporary tasks

**User-visible change:** Any custom Specialist can be selected for one dynamic
task; generic temporary tasks still require no Specialist.

**Primary files**

- Modify: `src-tauri/src/specialists.rs`
- Modify: `src-tauri/src/delegation_runtime.rs`
- Modify: `src-tauri/src/delegation_tool.rs`
- Modify Specialist/delegation DTOs and UI selectors.
- Update Specialists and delegation docs.

**Work**

1. Resolve optional `specialist_id` from the existing Specialist store.
2. Snapshot instructions, model binding, skill whitelist, and connector
   whitelist into the approved task spec.
3. Compose prompts in this order: base delegated-worker contract, Specialist
   identity, task/context/dependency inputs, result contract.
4. Intersect capability tools with Specialist skill/connector allowlists.
5. Respect the Specialist's model profile if valid; otherwise apply the
   existing active-model fallback and record the resolution.
6. Optionally add one generic executor preference field only if the UI needs a
   durable default. Do not add per-role backend special cases.
7. Reviewer remains available as an optional built-in Specialist. Do not append
   it automatically.
8. Include currently spawnable Specialist IDs/descriptions in the dynamic tool
   description without exposing their full private prompts to the parent model.

**Tests**

- Generic task works with no Specialist.
- Custom Specialist instructions/model reach the child.
- Specialist skill/connector whitelist narrows capability tools.
- Missing/deleted Specialist fails resolution before execution.
- Approved snapshot is unchanged after Specialist edit/removal.
- Reviewer runs only when requested.
- A code-capable custom Specialist runs natively without ACP.

---

## PR 9 — Complete native scientific capability adapters

**User-visible change:** Dynamic native tasks can use Wisp's scientific
workbench surfaces under explicit capability grants.

**Primary files**

- Modify the existing Tauri tool wiring helpers used by main sessions.
- Modify: `src-tauri/src/delegation_runtime.rs`
- Modify: `src-tauri/src/mcp_bridge.rs`
- Modify capability availability summaries.

**Work**

1. Reuse, rather than duplicate, the existing constructors/wiring for:
   - configured skills;
   - literature/custom MCP connectors;
   - Python/runtime tools;
   - ExecutionContext and Run Manager tools;
   - artifact registration;
   - image inspection/vision.
2. Capability availability is project/session-specific. Do not advertise a
   connector or runtime that is disabled/unconfigured.
3. `code_run` launches/queries structured Runs where work is long-lived.
4. Child outputs return durable resource references rather than embedding large
   datasets or binary content.
5. A Specialist allowlist and task capability grant must both permit a skill or
   connector.

**Tests**

- Fake connector/runtime registries produce the expected filtered child tool
  set.
- Disabled connector is not advertised and cannot be invoked.
- Long-running request creates a fake/mock Run record instead of extending
  shell timeout.
- Artifact/DataAsset/Paper references survive result persistence and parent
  delivery.
- No test requires real network, Python environment, SSH, WSL, scheduler, or
  GPU.

**Milestone B gate**

At this point temporary and Specialist-backed Agents can use Native or ACP,
all daily approval/UI flows are dynamic, and scientific capabilities use the
same policy model.

---

## PR 10 — Add persisted background completion and optional auto-resume

**User-visible change:** Long delegated batches can run in the background and
deliver one completion back to the owning conversation.

**Primary files**

- Modify store migration/init code only for fields requiring atomic delivery.
- Modify: `src-tauri/src/delegation_runtime.rs`
- Modify session runtime/message delivery code in `src-tauri/src/lib.rs`.
- Modify UI activity notifications.

**Work**

1. Add a conversation completion policy: inline (default) or background.
2. Background tool execution returns workflow/task handles immediately.
3. On terminal workflow state, atomically append one structured internal result
   to the owning frame and mark it delivered. Restart/retry must not duplicate
   it.
4. If auto-resume is enabled and the conversation is idle, resume the main
   Agent once to synthesize. If the user is actively sending another turn,
   queue the completion rather than racing it.
5. Cancellation and application shutdown leave explicit terminal/interrupted
   state; no unknown external process is silently resumed.
6. Full results remain readable even after delivery retention expires.

**Tests**

- Immediate handle, later exactly-once result injection.
- App restart between task completion and delivery.
- Retry produces a new attempt/result without duplicating the old delivery.
- Busy parent queues completion; idle parent auto-resumes once.
- Cancellation before/after child start.

---

## PR 11 — Add safe isolated parallel writers

**User-visible change:** Multiple writable tasks may run concurrently when each
uses an isolated Git worktree; non-Git projects remain serialized.

**Primary files**

- Add an isolation module with an injectable command runner.
- Modify native and ACP executor launch roots.
- Modify result/artifact persistence and draft UI.

**Work**

1. Implement only `git_worktree` isolation first:
   - verify Git repo and executable;
   - create a unique temporary worktree/branch with direct argument arrays;
   - run child inside it;
   - capture changed-file manifest and patch;
   - detect conflicts before apply/cherry-pick;
   - clean up on all terminal paths.
2. Approval shows isolation and merge behavior.
3. Parallel writable tasks require isolation. Without it they share the single
   mutation lane.
4. Non-Git projects fall back to serialized execution; they never silently
   receive unsafe parallel writes.
5. Preserve patches as Artifacts when automatic merge is rejected or conflicts.

**Tests**

- Temporary local Git repository covers independent changes, conflicting
  changes, cancellation, failed child, and cleanup.
- Fake command runner covers missing Git and platform-specific path handling.
- Windows path with spaces and macOS path behavior use argument vectors, not
  shell quoting.
- Non-Git fallback remains serialized.

Do not add overlayfs, APFS clone, ZFS, Btrfs, or ProjFS backends in this PR.

---

## PR 12 — Add bounded nested delegation

**User-visible change:** An approved child may create its own temporary batch
within root-wide limits.

**Primary files**

- Modify core plan/request lineage types.
- Modify workflow attempt persistence if indexed parent/root lookup is needed.
- Modify tool registration, executor context, scheduler budgets, and UI tree.

**Work**

1. Add root workflow ID, parent attempt ID, and depth to the resolved context.
2. Expose `delegate_tasks` to a child only when:
   - its task policy explicitly permits delegation;
   - current depth is below the root maximum;
   - root task/concurrency/token/tool/cost/time budgets have capacity.
3. Start with default max depth 1 and opt-in hard maximum 2.
4. Namespace child IDs under their parent for display/result lookup while
   preserving stable database IDs.
5. Propagate root cancellation and exhausted budgets to all descendants.
6. Roll nested structured results into the direct parent's result, then into
   the root parent.

**Tests**

- Allowed depth-2 fan-out and result rollup.
- Depth/task/budget limit prevents spawn before child creation.
- Root cancellation cancels descendants.
- A child cannot delegate merely by asking for raw tools or editing its prompt.
- Nested ID/result persistence survives restart.

Peer messaging remains out of scope. Add it only after a workflow demonstrates
that dependency results and durable project artifacts are insufficient.

**Milestone C gate**

Background, isolation, and bounded nesting are stable under cancellation,
restart, partial failure, and budget tests.

---

## PR 13 — Remove fixed-template delegation

**User-visible change:** Dynamic temporary Agents become the only delegation
mode. Fixed-template workflows and their legacy history are no longer exposed
or executable.

**Primary files**

- Remove `DelegationPlanner`, `AgentTemplateRegistry`, and fixed template lists.
- Remove fixed-team UI/i18n/docs.
- Remove the v1 parser/runner, legacy badges/actions, and Codex-specific
  delegation descriptions/checks left behind by the compatibility period.
- Update: `docs/agent-delegation.md`, README sections, and tests.

**Work**

1. Delete `propose_delegation` and its command/tool registration.
2. Remove `model_selected_templates`, fixed-template selection UI, automatic
   Reviewer append, and hard-coded code-first dependency construction.
3. Remove v1 deserialization, validation, retry/execution, legacy template
   commands, and legacy workflow rendering. Existing fixed-template records
   receive no migration and are intentionally unsupported after this PR.
4. Remove fixed-template fixtures and replace mixed-version branches with the
   single dynamic-policy path.
5. Update all user documentation to describe temporary dynamic tasks, parent
   result synthesis, Native default, and optional ACP.

**Tests**

- No command, tool, UI, or runtime path can create, display, retry, or execute a
  v1 fixed-template plan.
- Search assertions find no fixed built-in Agent names, legacy badges,
  `propose_delegation`, or v1 template validation paths outside historical
  planning documents.
- New v2 flows contain no built-in template IDs unless a Specialist is
  explicitly selected.
- Search assertion: no user-facing text says code-capable delegation requires
  Codex ACP.

---

## End-to-end acceptance matrix

The roadmap is not complete until this matrix is covered by automated or
documented manual tests.

| Persona | Capability | Executor | Topology | Expected |
| --- | --- | --- | --- | --- |
| temporary | reasoning/read | Native/current model | one task | inline result returns and parent synthesizes |
| temporary | read | Native/current model | two parallel | actual overlap under semaphore |
| temporary | read -> review | Native | dependency chain | structured dependency result injected |
| temporary | write/run | Native | serialized | approval, scoped mutation, no ACP installed |
| custom Specialist | read/literature | Native/Specialist model | parallel | persona/model/connector restrictions applied |
| temporary | code/run | selected non-Codex ACP | one task | ACP profile works without vendor check |
| Reviewer Specialist | review | Native or selected ACP | optional fan-in | only runs when requested |
| temporary | write | Native/ACP | isolated parallel | patches captured and conflicts safe |
| temporary | mixed success/failure | mixed eligible executors | DAG | unrelated branch completes; descendants block |
| temporary child | bounded capabilities | Native | depth 2 | root limits and result rollup enforced |
| any | long-running compute | Native Run tools | background | durable Run/result handle, exactly-once delivery |

## PR description checklist

Every PR in this series must state:

- the user-facing problem solved in that slice;
- the files and durable abstraction changed;
- tests added/updated and exact commands run;
- manual smoke steps for UI/platform behavior;
- known limitations and the numbered follow-up PR that addresses them;
- whether unsupported pre-dynamic records are affected.

## Explicitly deferred until evidence exists

- A proprietary generic HTTP Agent executor.
- Dynamic native libraries/plugins for arbitrary executors.
- Peer mailboxes/IRC between siblings.
- YAML workflow definitions separate from the persisted DAG.
- More than depth 2 or more than four concurrent children.
- Non-Git parallel-writer isolation backends.
- Semantic model slots such as fast/slow unless users need model-role routing;
  concrete model profiles and Specialist bindings cover the initial need.

These are not missing extension points. They are implementations deliberately
deferred until a concrete protocol, workload, or measured bottleneck exists.
