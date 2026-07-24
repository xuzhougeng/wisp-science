# ACP Agents

Wisp can run an external coding agent through the [Agent Client Protocol](https://agentclientprotocol.com/) (ACP v1) over local stdio.

ACP Agents are **not** HTTP model profiles. Settings → Models configures API providers for the built-in Wisp agent. ACP configures a separate local process that owns its own session, tools, and auth.

## Prerequisites

1. Install Node.js (needed for the official npm ACP adapters).
2. Install or authenticate the underlying agent the adapter wraps (Codex login / API key, Claude credentials, and so on).
3. Confirm the adapter starts from a terminal. It should wait on stdin for ACP JSON-RPC; that is expected.

Do **not** put plain `codex`, `claude`, or `claude -p` in the ACP form. Those CLIs are not ACP agents. Use an ACP adapter such as:

- [`@agentclientprotocol/codex-acp`](https://github.com/agentclientprotocol/codex-acp)
- [`@agentclientprotocol/claude-agent-acp`](https://github.com/agentclientprotocol/claude-agent-acp)

## Where to find it

Under **Settings → Models** there are two categories with the same list → add/edit flow:

1. Open **Settings → Models**
2. Switch tabs: **Models (n)** | **ACP Agents (n)**
3. Click **Add model** or **Add ACP Agent** (breadcrumb subpage form)
4. Or from the chat model picker: **Add model** / **Add ACP Agent** — same forms

Click a row to edit. HTTP and ACP both use Cancel / Save on the subpage.

## Add an ACP Agent in Wisp

1. Open a project. You may start from an empty session or an existing
   conversation; selecting ACP from a populated conversation automatically
   starts a new empty session while preserving the composer draft.
2. Open the ACP dialog with one of the paths above.
3. Fill the form:

| Field | Meaning |
| --- | --- |
| Label | Display name in the picker (for example `Codex ACP`) |
| Command | Executable only — no shell quoting, no combined `cmd args` string |
| Arguments | One argument per line |

4. Click **Save Agent**.
5. Click **Test Connection**. A success message means Wisp could launch the process and complete ACP `initialize`.
6. If auth methods appear after the test, click the advertised button (for example browser login). Credentials stay with the agent; Wisp does not store them in SQLite.
7. Close the dialog, open the model picker again, and select the ACP Agent under **ACP Agents**.
8. Send a prompt. The first prompt locks that session to the selected agent.

To switch back to a normal HTTP model profile, start another empty session and pick a Models entry instead.

## Example profiles

Wisp launches `Command` plus each Arguments line as a process argv. Put every shell token on its own line.

### Codex ACP via `npx`

Install/check once:

```bash
npx -y @agentclientprotocol/codex-acp --version
```

In Wisp:

| Field | Value |
| --- | --- |
| Label | `Codex ACP` |
| Command | `npx` (on Windows prefer `npx.cmd`, or the full path to `npx`) |
| Arguments | `-y`<br>`@agentclientprotocol/codex-acp` |

Global install alternative:

```bash
npm install -g @agentclientprotocol/codex-acp
```

| Field | Value |
| --- | --- |
| Label | `Codex ACP` |
| Command | `codex-acp` (or the absolute path returned by `where codex-acp` / `which codex-acp`) |
| Arguments | _(empty)_ |

Optional env for the agent process (set in your OS / shell before launching Wisp):

- `CODEX_API_KEY` or `OPENAI_API_KEY`
- `CODEX_PATH` if you want a specific Codex binary

### Claude Agent ACP via `npx`

```bash
npx -y @agentclientprotocol/claude-agent-acp --version
```

| Field | Value |
| --- | --- |
| Label | `Claude ACP` |
| Command | `npx` / `npx.cmd` |
| Arguments | `-y`<br>`@agentclientprotocol/claude-agent-acp` |

Or:

```bash
npm install -g @agentclientprotocol/claude-agent-acp
```

| Field | Value |
| --- | --- |
| Label | `Claude ACP` |
| Command | `claude-agent-acp` |
| Arguments | _(empty)_ |

## Using an ACP session

- Select the agent on an empty session, then chat normally.
- Selecting an ACP Agent from a conversation that already has messages creates
  and opens a fresh ACP session automatically. Existing transcript history is
  left unchanged because ACP cannot bind it as native session history.
- Permission cards show the exact options the agent returns; choose one to continue.
- If the agent advertises session config options (model, mode, …), they appear above the composer for that turn.
- Stop cancels the active ACP turn for the bound session.
- After restart, Wisp reconnects only when the same profile fingerprint and project path still match and the agent supports resume/load. Editing Command/Arguments creates a new fingerprint; start a fresh session.

Wisp injects its scientific MCP bridge into the ACP session, so the external
agent can call bundled Wisp tools while it works in the project directory. The
bridge exposes the following project-scoped Wisp Harness gateway:

This full bridge description applies to a user-owned ACP chat session. A
temporary delegated ACP task is narrower: it receives no bridge by default and
only the individual gateway tools granted by that task's resolved capabilities.

- `wisp_get_capabilities` — inspect the exact grant and current limitations
- `wisp_list_skills` / `wisp_use_skill` — discover and load enabled skills
- `wisp_search_tools` / `wisp_use_tool` — discover and call scientific or custom MCP tools without loading the full schema catalog
- `wisp_search_memory` — read durable project memory
- `wisp_list_artifacts` — list artifacts owned by the active project
- `wisp_get_research_graph` — read project research nodes and edges
- `wisp_list_execution_contexts` — read context capabilities and probe status
- `wisp_run_in_context`, `wisp_get_run`, `wisp_monitor_run`, and `wisp_cancel_run` — persisted Run controls; `wisp_monitor_run` waits without repeated model polling
- enabled scientific tools and custom MCP connections, available through the search/use pair above

This is deliberately a capability gateway, not an unrestricted export of every
internal Rust object or UI command. Memory/artifact/graph writes and persistent
runtime mutation are not exposed until Wisp has an ACP approval broker. Context
connection configuration is redacted, and all artifact/graph reads remain
scoped to the active project. The ACP process can still use its own filesystem
tools with the OS permissions described under **Current limits**.

Composer references work in ACP sessions too:

- `/` adds the selected enabled skill's rendered `SKILL.md` guidance to that
  ACP prompt as text.
- `#` adds the selected session transcript as reference-only text, with the
  same size limits and prompt-injection guard as Wisp's built-in agent.
- `@` sends the selected artifact as a standard ACP file link. Cross-project
  artifacts remain at their original validated local path.

## Reviewing ACP sessions

The Reviewer specialist can review both built-in HTTP-agent sessions and ACP
sessions. Automatic review now runs after a qualifying ACP turn, persists the
report, and can send one correction turn back to the original ACP session when
findings are present. Manual **Review** uses the same backend selection.

Reviewer backend choices are:

- **Default HTTP model** — the active/default Wisp HTTP model profile
- **Follow session** — an HTTP session uses its HTTP default; an ACP session
  launches a separate one-shot reviewer with the same ACP profile
- a specific **HTTP model** profile
- a specific **ACP Agent** profile, launched as a separate read-only one-shot
  reviewer session

The reviewer never shares the original ACP session state. It reviews the
persisted transcript and cannot request tool permissions. ACP tool snapshots
also persist standard `rawInput` and `rawOutput` evidence when the adapter
provides them. If an adapter records only a terminal handle/status and no
inspectable output, Wisp reports the result as **Unreviewable** with an evidence
coverage warning instead of incorrectly showing a green pass. Reviewer launch,
API, timeout, or JSON parsing failures are shown in the chat rather than
silently disappearing. One-shot ACP reviewer calls time out after 90 seconds
and can be cancelled with the active turn. Automatic correction instructions
remain control-plane messages instead of being added to the user-authored
conversation history.

## Importing Codex CLI conversations

Conversations run in the standalone Codex CLI (outside Wisp) can be imported
into the current project, so Codex work continues in Wisp without copy/paste
(#464).

- Open it from the sidebar: the download icon next to the **Sessions** header.
- The dialog lists local rollouts from `~/.codex/sessions` (newest first) with
  the working directory, message count, and last activity.
- **Import** copies the user/assistant turns into a regular Wisp session; the
  original Codex chronology is preserved in the sidebar ordering.
- Re-importing is idempotent. If the Codex side gained new turns since the
  last import, the row shows **Update** and importing fast-forwards the
  session; a session that was continued inside Wisp is left untouched.
- Codex context plumbing (AGENTS.md preamble, `<environment_context>` wrappers,
  tool call records, reasoning items) is filtered out — only the conversation
  itself is imported.

## Troubleshooting

| Symptom | Likely fix |
| --- | --- |
| Test Connection fails immediately | Command not on PATH, wrong Windows wrapper (`npx` vs `npx.cmd`), or Arguments still glued into Command |
| Auth button fails | Finish login/API key setup for the underlying agent outside Wisp, then retest |
| “selection is locked after the first prompt” | Expected; create a new empty session to change backend |
| “profile or project path changed” | Profile Command/Arguments or project cwd changed; start a new ACP session |
| Agent runs but has no science tools | Confirm the session started through Wisp (MCP bridge is injected automatically) |
| Agent does not call a bridge tool | Verify the selected ACP adapter supports MCP servers; the bridge tools are available to the agent, but its model decides when to invoke them |
| Review says Unreviewable | The ACP adapter did not persist inspectable tool output. Upgrade/configure the adapter to emit `rawOutput`, then run the task and review again |
| ACP reviewer fails to start | Test that ACP profile under Settings first and complete the adapter's authentication flow |
| Reviewer backend shows Missing ACP Agent | The saved reviewer profile was removed. Select and save another ACP Agent or an HTTP reviewer backend |
| ACP reviewer times out | The one-shot reviewer exceeded 90 seconds. Retry it or choose a faster reviewer backend; the primary answer remains available |
| A local script says Preview is not supported | In-app source preview currently supports `.R`, `.py`, and `.sh`; open or download other file types with an external application |

## Current limits

- Local stdio only — no remote / WSL / SSH ACP transport yet
- No in-app ACP registry installer — configure an already-installed agent command
- No ACP rewind/fork, image/audio prompt blocks, or client-provided terminal/filesystem in this release
- Harness writes (memory, artifacts, research graph, persistent runtime) are not yet exposed through ACP
- The local agent process has the OS permissions of the Wisp user

## Related docs

- [GitHub Pages: ACP Agent 配置](acp-agents.html) — site page for this guide
- [Model configuration](model-configuration.md) — HTTP API profiles for the built-in agent
- [ACP client integration plan](superpowers/plans/2026-07-11-acp-client-integration.md) — architecture notes
- [ACP protocol](https://agentclientprotocol.com/protocol/v1/overview)
