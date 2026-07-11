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

## Add an ACP Agent in Wisp

1. Open a project and start a **new empty** session.
2. In the composer model picker, click **Add ACP Agent**.
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
- Permission cards show the exact options the agent returns; choose one to continue.
- If the agent advertises session config options (model, mode, …), they appear above the composer for that turn.
- Stop cancels the active ACP turn for the bound session.
- After restart, Wisp reconnects only when the same profile fingerprint and project path still match and the agent supports resume/load. Editing Command/Arguments creates a new fingerprint; start a fresh session.

Wisp also injects its scientific MCP bridge into the ACP session, so the external agent can call bundled Wisp tools while it works in the project directory.

## Troubleshooting

| Symptom | Likely fix |
| --- | --- |
| Test Connection fails immediately | Command not on PATH, wrong Windows wrapper (`npx` vs `npx.cmd`), or Arguments still glued into Command |
| Auth button fails | Finish login/API key setup for the underlying agent outside Wisp, then retest |
| “selection is locked after the first prompt” | Expected; create a new empty session to change backend |
| “profile or project path changed” | Profile Command/Arguments or project cwd changed; start a new ACP session |
| Agent runs but has no science tools | Confirm the session started through Wisp (MCP bridge is injected automatically) |

## Current limits

- Local stdio only — no remote / WSL / SSH ACP transport yet
- No in-app ACP registry installer — configure an already-installed agent command
- No ACP rewind/fork, image/audio prompt blocks, or client-provided terminal/filesystem in this release
- The local agent process has the OS permissions of the Wisp user

## Related docs

- [Model configuration](model-configuration.md) — HTTP API profiles for the built-in agent
- [ACP client integration plan](superpowers/plans/2026-07-11-acp-client-integration.md) — architecture notes
- [ACP protocol](https://agentclientprotocol.com/protocol/v1/overview)
