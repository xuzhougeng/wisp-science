# Controlled Agent delegation

The **Agents** tab in the right panel turns a project goal into a persisted,
reviewable multi-Agent workflow. It is separate from choosing an ACP Agent as
the model for a normal conversation.

## Workflow

1. Open a project and the right panel, then choose **Agents**.
2. Describe a code, analysis, biology, or visualization goal and choose Manual,
   Assisted, or Automatic mode.
3. Create the draft. Review each step's backend, tools, token budget, and
   timeout. A draft can be edited and regenerated without changing an approved
   plan behind the user's back.
4. Approve the immutable plan, then run it.
5. Follow persisted step attempts and usage in the panel. Cancel requests are
   stored in SQLite, so the scheduler and both local and ACP backends observe
   the same state.
6. Failed or cancelled workflows can be returned to Approved with **Retry**.
   Completed step sessions can be opened with **Take over** for ordinary chat.

## Safety and current limits

- At most two delegated steps run concurrently. Dependencies are respected and
  a final Reviewer runs only after its inputs succeed.
- Templates cap tools, project paths, context, time, tokens, tool calls, and
  cost. Delegated Agents cannot delegate again.
- Code-capable ACP steps require a configured Codex ACP profile. Codex runs in
  workspace-write mode with command network access disabled. Its effective
  approval policy is `on-request`; Wisp rejects command, process, MCP, network,
  and unscoped file escalations at the ACP boundary.
- Wisp stores attempts, structured results, evidence, artifacts, usage, child
  conversation IDs, and ACP session IDs. API keys and private keys remain in
  their existing credential stores and are not copied into workflow records.
- Application shutdown marks interrupted workflows failed. Use **Retry** after
  inspecting the recorded error; Wisp does not silently resume an unknown
  external process.

The current planner is intentionally small and template-based. It recognizes
code/analysis, biology, and visualization goals; unrelated simple goals stay in
the main conversation.
