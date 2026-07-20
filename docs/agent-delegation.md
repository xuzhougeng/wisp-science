# Agent delegation

Delegation lets the main Agent create a bounded set of temporary sub-Agents,
run independent work in parallel, and synthesize their evidence either in the
same turn or after durable background completion. Codex and ACP are optional
executor choices; neither is part of the meaning of a code-capable Agent.

## Inline temporary Agents

1. Open the composer Agent menu and enable **Delegation** for the current
   conversation. New conversations start with delegation off.
2. Ask the main Agent for an outcome that materially benefits from independent
   or parallel work. The main Agent decides whether delegation is useful and,
   when it is, calls `delegate_tasks` itself.
3. The call describes an overall goal, bounded shared context, and up to eight
   tasks. Each task has its own instruction, dependency IDs, capability IDs,
   optional Specialist, optional JSON output schema, and optional isolation
   request.
4. Wisp resolves every capability through host policy into an exact model,
   executor, tool set, project scope, workspace policy, budget, and timeout.
   The model cannot grant raw tools or permissions to a child.
5. Safe read-only tasks run immediately. A batch that can write, execute code,
   use an external service, request isolation, or exceed normal budgets uses
   the existing approval prompt. Rejecting it starts no child and returns the
   feedback to the main Agent so it can revise the batch.
6. Independent tasks run concurrently up to the batch limit. A dependent task
   starts only after its direct dependencies succeed and receives their
   structured results. An unrelated branch continues after another branch
   fails; only descendants of the failed branch are blocked.
7. Ordered, compact results return as tool output. The main Agent must combine
   them into its final response rather than sending the user elsewhere. If a
   result was truncated, `get_delegated_result` reads that task's full persisted
   result for the same conversation.

Omitting `specialist_id` creates a generic temporary Agent. Selecting a
Specialist reuses its persona, model preference, skills, and connector
restrictions as an immutable snapshot for that run. A Specialist is therefore
an optional preset, not a required fixed team slot. The parent Agent sees only
the currently available Specialist IDs, names, and descriptions; private
instructions are copied into the selected child snapshot, not exposed in the
`delegate_tasks` description. The child prompt is composed from the bounded
worker contract, Specialist identity/instructions, task context and dependency
inputs, then the result contract.

A valid Specialist model preference is used when the task resolves to Native.
An empty or deleted model binding falls back through the normal active-model
selection and the resolved model is persisted. ACP profiles remain executor
choices rather than Specialist model bindings. The built-in Reviewer follows
the same optional selection rule and is never appended to a dynamic plan
automatically.

## Background completion

The composer Agent menu has a per-conversation **Completion** setting. Inline
is the default and preserves the same-turn behavior above. Background returns
a workflow handle as soon as the approved batch is scheduled, allowing the
parent turn and the rest of the app to continue. The main Agent must not poll
that handle.

Workflows started directly from the Agents panel are already detached from a
parent model turn, so they always use the durable background delivery path.
The conversation's auto-resume setting still decides whether their parent is
automatically synthesized.

Each background execution reserves a persisted generation before any child
starts. When the workflow reaches succeeded, failed, or cancelled, Wisp stores
one compact result for that generation. Under the same conversation lock used
by normal turns, it then atomically appends one internal result message and
marks the generation delivered. A busy parent finishes its current or already
queued user turn first; this prevents background delivery from racing the
turn's incremental message sequence. Retrying a failed or cancelled workflow
creates a new generation, so the retry can deliver once without redelivering
the earlier result.

When **Background** is selected, enable **Auto-resume parent** to let an idle
parent Agent synthesize newly delivered results without another user message;
the option is hidden for inline completion, where it does not apply. Several
completions that become ready together may be combined into one synthesis
turn, but each generation's resume claim is made only once. If the app stops
after claiming that turn, the
claim is recorded as interrupted instead of being silently replayed on restart.
Without auto-resume, the completion card remains in the owning conversation
and enters the Native parent's context on its next turn. ACP parents receive
the same result as internal context on their next prompt because their own
transcript is maintained by the external Agent.

On startup, queued/running child attempts become explicit failed attempts, and
a background generation reserved before its first child started becomes an
explicit failed workflow. Terminal generations that were persisted just before
a crash are reconstructed from their immutable plan and attempts, then
delivered normally. The compact conversation message may later be removed by
ordinary transcript retention; full task responses and lookup records remain
in workflow attempts.

## Native, ACP, and code execution

Native execution runs the ordinary Wisp Agent loop in a separate child
conversation with only the resolved tools. It supports project reading,
project writing, and bounded Run Manager execution without starting an ACP
client. This is the default eligible executor and is enough for a code task.

Scientific resources are resolved for the owning project and conversation at
draft time, then checked again before execution. Wisp considers the project's
enabled Skills, enabled bundled/custom MCP connections, selected
ExecutionContexts, configured Python/R interpreters, runtime workers, and
vision-capable models. A disabled or missing resource is omitted from both the
editor and `delegate_tasks` schema instead of being advertised optimistically.
Changing this resource set invalidates an already approved authorization
snapshot, so the task must be reviewed against the new authority.

The initial resource mapping is deliberately capability-shaped:

- `literature_search` grants only enabled literature Skills and literature
  connectors.
- `external_research` grants only enabled non-literature MCP connections.
- `visualization` grants configured Python/R tools and figure-oriented Skills.
- `code_run` grants `run_in_context`, `get_run`, and `cancel_run`. A generic
  temporary code task does not inherit every project Skill; a selected
  Specialist may reuse its configured non-literature Skill set.
- `image_inspection` grants local image reading only when the selected Native
  model supports vision.

For every task, its capability grant and its immutable Specialist whitelist
must both allow a Skill or connector. `None` on a selected Specialist keeps
the existing “inherit project settings” behavior; an explicit list narrows it.
The resulting exact resource IDs are installed directly in a Native child or
encoded as private allowlist tokens for that ACP child's filtered Wisp MCP
bridge. They are not inferred from an ACP vendor, command name, or Agent label.

ACP profiles remain available to workflows that explicitly resolve to an ACP
executor. Every configured profile whose command is currently available is
listed separately, and the selected profile ID—not its command, label, model,
or task name—controls routing. A profile may use Codex or another compatible
Agent, but the task is still defined by capabilities and contracts, not by an
ACP or Codex template. Automatic selection continues to prefer Native whenever
Native satisfies the task; choosing ACP is an explicit, approval-visible
override.

Delegated ACP sessions start with no Wisp MCP bridge. Wisp adds only bridge
tools implied by the resolved task permission set; for example, `code_run` can
receive the project-scoped execution-context and Run Manager tools while a
reasoning or file-read task receives no bridge. ACP permission requests are
matched against the same resolved tools, write flag, and project path ceiling,
independent of the ACP vendor. Unknown command, process, MCP, and network
requests are rejected.

Long-lived code is submitted as a persisted Run rather than by increasing the
delegated shell timeout. The child receives the conversation's selected remote
contexts plus the always-available local context, and can query or cancel the
Run by ID. Direct `shell` is never registered for a delegated Native child;
ACP receives the same Run control plane through the filtered bridge.

When a child links a project-local output in its structured summary or
evidence, Wisp snapshots the file as a content-addressed Artifact and returns
its durable ID with the task result. Structured DataAsset and Paper references
remain JSON references in the persisted response and parent delivery; large
or binary payloads are not copied into the conversation. A configured custom
MCP connection is treated as available from its saved configuration, but a
connection failure at execution is still reported by the child because Wisp
does not perform network health checks while drafting.

The same inline delegation surface is exposed through the Wisp MCP bridge as
`wisp_delegate_tasks` and `wisp_get_delegated_result` when the owning
conversation opted in. Because that bridge is non-interactive, a batch that
requires approval is denied instead of silently escalating.

## Bounded nested delegation

Nesting is opt-in per task through the `delegation` capability. A task that was
not resolved with that capability never receives `delegate_tasks`, even if its
prompt asks for it or a stored/raw tool name is forged. An authorized Native
or ACP child receives the same dynamic task protocol with authority narrowed
to its own capability, model, executor, permission, context, budget, and
timeout snapshot.

The default root limit remains one Agent level. Selecting `delegation` raises
that workflow to the hard maximum depth of two: a root child may create one
temporary child batch, and a depth-two child cannot delegate again. Root-wide
limits cover at most eight total tasks, two concurrent active children, and
the aggregate token, tool-call, cost, and wall-clock budgets. Registration and
attempt start reserve these limits atomically before a backend child or ACP
process is created. While an authorized parent waits synchronously for its
children, it yields its concurrency slot and reacquires it before continuing.

Nested task display IDs are namespaced under the parent, such as
`analysis/check_data`, while database workflow, step, and attempt IDs remain
stable. Root cancellation, deadline expiry, and budget exhaustion propagate to
every descendant. Completed nested batches are stored as structured results on
the direct parent response and are included again in the compact root result,
so synthesis does not depend on parsing a child transcript. Lineage and result
lookup survive application restart. Peer-to-peer sibling messaging is not part
of this model; dependencies, persisted artifacts, and parent result rollup are
the coordination paths.

## Persistence and safety

- Wisp persists the resolved v2 plan before execution. Stored steps contain the
  immutable Specialist, requested model/executor preferences, capability
  revisions, resolved permissions/model/executor, contracts, budgets, and
  policy integrity hash used for revalidation. ACP tasks do not store a
  decorative Native model that the ACP process would ignore.
- Background executions persist a generation and completion intent before
  launch. Result insertion, conversation delivery, auto-resume claim, and
  resume outcome are separate durable states; application restart never
  guesses that an unknown external process is still running.
- Before approval, a v2 draft exposes both its editable proposal and the
  resolved authority that will actually run. Each edit checks the draft's
  version, reruns dependency and policy resolution, and replaces the plan
  atomically. Approval makes the snapshot immutable; run and retry reuse that
  exact snapshot instead of asking a planner to recreate it.
- Read-only tasks may share the project workspace. Writable or executable
  tasks without isolation use one mutation lane and cannot edit the same
  checkout concurrently. When Git is installed and the project checkout is
  clean, a task may instead use a unique temporary Git worktree and run in
  parallel with other isolated writers. The approval card shows that Wisp will
  conflict-check and then cherry-pick the task's temporary commit.
- Native and ACP children both receive the isolated project root. Wisp captures
  a changed-file manifest and binary-capable patch, serializes merge decisions,
  and removes the temporary worktree and branch on success, failure, or
  cancellation. A failed child is never merged. A rejected/conflicting merge
  leaves the main checkout unchanged and stores the patch as an Artifact.
  Child-declared artifacts are copied to durable app storage before worktree
  cleanup, so their paths do not expire with the temporary directory. If a
  declared local artifact cannot be retained, the task fails and its project
  patch is not merged.
- Native Python/R kernels use a request-scoped runtime namespace instead of a
  project-wide kernel while isolated. Worktree finalization waits for Runs
  owned by that child; a failed/cancelled child cancels them. Timeout/drop
  cleanup cancels remaining child Runs and stops its runtime before removing
  the worktree.
- Non-Git and dirty project checkouts do not advertise isolation. Ordinary
  writable tasks still work there through the serialized mutation lane; an
  explicit isolation request fails closed instead of silently weakening its
  workspace guarantee. Initial isolation intentionally does not copy ignored
  files or add overlayfs/APFS/ZFS/ProjFS backends.
- Children receive only their instruction, bounded shared context, applicable
  project instructions, explicit inputs, and direct dependency results. They
  do not receive the full parent transcript.
- Delegated Agents receive `delegate_tasks` only from an approved `delegation`
  capability and only while root-wide depth, task, concurrency, token, tool,
  cost, cancellation, and time checks still have capacity.
- Output contracts are checked before results reach the parent. Attempts,
  structured results, artifacts, evidence, usage, child conversation IDs, and
  backend session IDs remain auditable in SQLite. Secrets stay in the existing
  credential stores.
- Turning Delegation off prevents the main conversation and its MCP bridge from
  listing or invoking delegation tools. It does not erase workflow history or
  implicitly cancel a workflow that is already running.

## Dynamic Agents panel

The right-panel Agents view is the control and audit surface for both inline
and manually drafted work. It groups workflows by their owning conversation.
Nested workflows appear indented beneath the root workflow with their depth
and namespaced task IDs. They are execution records rather than independent
drafts, so edit, approve, run, and retry controls remain on the root only.
Each dynamic task shows dependencies, requested capabilities, optional
Specialist, resolved model and executor, workspace/tool authority, approval
reasons, status, duration, usage, summary, and whether a full result is
available. **Inspect result** opens the persisted structured response; **Take
over** opens that task's child conversation.

The editor creates arbitrary temporary tasks instead of assembling a fixed
team. Add up to eight bounded tasks, connect them with dependencies, and
choose capabilities from the live policy registry. Advanced controls can
request a Specialist persona, model, eligible executor, isolation, budgets,
and a JSON output schema. UI-authored drafts pass through the same resolver as
main-Agent-authored batches, so the form never grants raw tools or authority.
Turning Delegation off disables new drafts, approvals, runs, and retries while
leaving supported dynamic history and cancellation available.

Only schema-version-2 dynamic plans are part of the product surface. Earlier
fixed-plan records are not migrated or deleted, but the Agents panel does not
list them and workflow actions reject them before approval, retry, or execution.

## Manual smoke check

Enable Delegation and ask the main Agent to compare two project files using two
independent temporary Agents. Confirm in the Agents panel that the two root
tasks overlap, their dependent synthesis task waits, and the final chat
response contains one synthesized comparison. Switch **Completion** to
**Background**, repeat the request, and verify the initial tool result is a
running handle followed later by exactly one completion card in the same
conversation. Enable **Auto-resume parent** and verify an idle conversation
adds one synthesized assistant update; start another parent turn and verify a
completion waits behind it. Then create an equivalent draft with **Add task**
and confirm no predefined team is required. Repeat with a write capability:
Wisp should show the exact resolved authority and start zero children if
approval is denied.
Finally, add the **Nested delegation** capability to one root task and let it
create two independent leaf tasks. Confirm that both leaves appear under the
same root card at depth 2, their IDs are prefixed by the parent task, their
structured results appear in the root result, and cancelling the root marks
the parent and both leaves for cancellation.
For the isolation path, start from a clean Git project and create two independent
write tasks with **Use an isolated workspace** enabled. Confirm that approval
shows **Conflict-check, then cherry-pick**, both children overlap, both changes
land as separate commits, and no `wisp-agent/*` worktree branch remains. Then
make both tasks edit the same line and confirm one merge is rejected, the main
file keeps the accepted change, and the rejected patch is available as an
Artifact. Cancel another isolated writer and confirm its partial patch is
preserved without modifying the main checkout.
