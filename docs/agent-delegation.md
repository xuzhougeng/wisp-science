# Controlled Agent delegation

The **Agents** tab in the right panel turns a project goal into a persisted,
reviewable multi-Agent workflow. It is separate from choosing an ACP Agent as
the model for a normal conversation.

## Workflow

1. Open the composer Agent menu and enable **Delegation** for the current
   conversation. New conversations start with delegation off.
2. Open the right panel and choose **Agents**, or ask the main Agent to propose
   a delegated plan. The main Agent can only create a persisted draft; it
   cannot approve or run the plan on the user's behalf.
3. Describe a code, analysis, biology, or visualization goal and choose a mode:
   - **Manual** requires an explicit ordered Agent team. The selected steps run
     sequentially, and Reviewer is optional but must be last.
   - **Assisted** asks the active model to select the smallest useful team from
     the controlled templates using the recent conversation as context. It
     persists a draft for review and never starts work automatically.
   - **Automatic** uses the same model-backed planning step, then approves and
     starts a low-risk local read-only plan in the background. Plans involving
     ACP, file writes, network access, or a larger budget pause as a draft and
     require **Approve and run**.
4. A generated card is a plan, not an Agent result. Review each step's backend,
   tools, token budget, and timeout. A draft can be edited and regenerated
   without changing an approved plan behind the user's back.
5. In Manual or Assisted mode, approve the immutable plan and run it. Automatic
   mode starts as described above.
6. Follow persisted step attempts and usage in the panel. Cancel requests are
   stored in SQLite, so the scheduler and both local and ACP backends observe
   the same state.
7. Failed or cancelled workflows can be returned to Approved with **Retry**.
   Completed step sessions can be opened with **Take over** for ordinary chat.

## Safety and current limits

- Assisted and Automatic workflows run at most two delegated steps concurrently.
  Manual workflows run their explicit order sequentially. Dependencies are
  respected and a generated final Reviewer runs only after its inputs succeed.
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
- Turning Delegation off blocks new drafts, approvals, runs, and retries for
  that conversation. It does not hide history or cancel an already running
  workflow; cancellation remains an explicit action in the Agents panel.

Model-backed planning can only select the built-in code execution, biology
interpretation, and visualization templates; Wisp constructs and validates the
actual Agent specs and appends Reviewer. The model cannot invent tools,
permissions, backends, or nested delegation. Unrelated simple goals can remain
in the main conversation.
