# Hooks

Settings → Hooks / 钩子 lists everything that runs at agent lifecycle events.

## Default hooks

| Hook | Event | Setting |
| --- | --- | --- |
| Auto-review / 自动审查 | Stop | Default for new conversations. The composer menu still switches it per conversation. **Reviewer model** opens the Reviewer specialist. |
| Analyze tool failures / 自动分析工具失败 | Stop (after the turn) | Global switch, failure-rate threshold and minimum failures. Same setting as the composer menu. |

## Command hooks

A command hook is a shell command that runs at one event. It runs in the
project folder (PowerShell on Windows, `sh` elsewhere) with a 60 s timeout,
or the hook's own `timeout` in seconds. There are two sources, and both run:

1. **Your hooks.** **New hook / 新建钩子** on the Hooks page stores them in
   the app settings. They apply to every project.
2. **Project hooks.** `<project>/.wisp/hooks.json` is checked into the
   project. These hooks run after yours, and only once you trust the file.

| Event | When | Block (exit 2 or `"decision": "block"`) |
| --- | --- | --- |
| `UserPromptSubmit` | Before a message is sent | Refuses the message |
| `PreToolUse` | Before a tool call, ahead of approvals | Blocks the call; the reason is the tool result |
| `PostToolUse` | After a tool call succeeds | Appends the reason to the result the agent sees |
| `PostToolUseFailure` | After a tool call fails | Appends the reason to the result the agent sees |
| `Stop` | When the agent finishes a turn | Continues the turn once with the reason as the instruction |

Tool events take an optional **tool matcher**: a tool name or anchored regex
such as `shell|write|edit` or `mcp:.*`. MCP tools match by their full
`mcp:<name>` name. Empty or `*` matches every tool. Tool events only fire for
the built-in agent; ACP agents run their own tools, so they only see
`UserPromptSubmit` and `Stop`.

### Input

The command receives JSON on stdin, with Claude Code / Codex field names:

```json
{
  "session_id": "…",
  "cwd": "/path/to/project",
  "hook_event_name": "PreToolUse",
  "tool_name": "shell",
  "tool_input": { "cmd": "rm -rf build" }
}
```

`UserPromptSubmit` adds `prompt`. `PostToolUse` and `PostToolUseFailure` add
`tool_response: { success, content }`. `Stop` adds `stop_hook_active`, which is
true on the re-check after a continuation.

### Output

- **Exit 2** blocks, with stderr as the reason.
- **Exit 0** continues. stdout may be a JSON decision, as in Claude Code:

  ```json
  { "decision": "block", "reason": "Run the tests first." }
  { "hookSpecificOutput": { "permissionDecision": "ask", "permissionDecisionReason": "Writes to /data" } }
  { "hookSpecificOutput": { "additionalContext": "ruff fixed 2 issues" } }
  ```

  `decision` / `permissionDecision` can be `block` or `deny` (block), `ask`, or
  `allow` (continue). `additionalContext` is added to turn context for
  `UserPromptSubmit` and sent to the agent after a tool. stdout that is not
  JSON is turn context for `UserPromptSubmit` and ignored for other events.
- **Anything else** (another exit code, invalid JSON, a timeout or a launch
  failure) shows a status-line notice. `PreToolUse` **fails closed**: the call
  is blocked, so a broken or hung policy hook never lets a tool through. The
  other events continue.

The first hook that blocks wins; later hooks for that event do not run, so a
project hook cannot override a block from your own hooks.

### Hooks and approvals

`PreToolUse` runs before the tool's approval gate. A hook can only make a call
stricter:

- `block` / `deny` skips the call, even under Full Permission.
- `ask` treats the call as if its approval were **Ask**. Full Permission and a
  **Deny** setting still win, as they do for a per-tool Ask.
- `allow` never skips an approval prompt or lifts a Deny.

### Stop hooks and the Reviewer

When a turn completes, automatic review runs first (one correction at most),
then the Stop command hooks. If a Stop hook blocks, the agent keeps working
once with the reason. The Stop hooks then run again with
`stop_hook_active: true`. If one still blocks, the status line says so and the
turn ends. It does not loop.

## Project hooks (`.wisp/hooks.json`)

The file uses the Claude Code / Codex layout, so existing hooks can be copied
in. Tool names are Wisp's (`shell`, `write`, `edit`, `mcp:…`), not Claude
Code's (`Bash`, `Write`, …).

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "shell",
        "hooks": [
          { "type": "command", "command": "python3 .wisp/hooks/deny_destructive_shell.py", "timeout": 10 }
        ]
      }
    ],
    "Stop": [
      { "hooks": [{ "type": "command", "command": "python3 .wisp/hooks/verify_outputs.py", "timeout": 30 }] }
    ]
  }
}
```

Events Wisp does not fire (for example `SessionStart`) and handlers whose
`type` is not `command` are skipped.

**Trust.** Opening a project never runs its hooks. The Hooks page shows the
project's file under **This project / 当前项目**, with every command in full.
**Trust and enable / 信任并启用** records the file's SHA-256 for this project.
Hooks run only while the file still has that exact content. Any edit makes it
untrusted until you review it again, and trust can be revoked at any time. While
a project has an untrusted or invalid file, the first hook event shows a
status-line notice once, pointing to the Hooks page.

Trust covers `hooks.json` itself, not the scripts it calls. A command such as
`python3 .wisp/hooks/check.py` runs whatever that script contains, as in
Claude Code and Codex.

## Environment

Hooks inherit the app's environment, like the `shell` tool, plus the
configured command proxy. API keys and SSH keys live in the OS keyring and are
never in that environment.

## Not supported yet

- `PermissionRequest`, `SessionStart` and `PreCompact` events.
- Hooks bundled with plugins.
- A timeout stops the hook's own process, not processes it started.
