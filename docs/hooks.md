# Hooks

Settings → Hooks / 钩子 lists everything that runs at agent lifecycle events.

## Default hooks

| Hook | Event | Setting |
| --- | --- | --- |
| Auto-review / 自动审查 | Stop | Default for new conversations. The composer menu still switches it per conversation. **Reviewer model** opens the Reviewer specialist. |
| Analyze tool failures / 自动分析工具失败 | Stop (after the turn) | Global switch, failure-rate threshold and minimum failures. Same setting as the composer menu. |

## Command hooks

**New hook / 新建钩子** adds a shell command for one event. Hooks are stored in
the app settings (user scope) and run in the project folder: PowerShell on
Windows, `sh` elsewhere, with a 60 s timeout.

| Event | When | Exit 2 |
| --- | --- | --- |
| `UserPromptSubmit` | Before a message is sent; stdout (exit 0) is added as turn context | Refuses the message |
| `PreToolUse` | Before a tool call, ahead of approvals | Blocks the call; stderr is the tool result |
| `PostToolUse` | After a tool call succeeds | Appends stderr to the result the agent sees |
| `PostToolUseFailure` | After a tool call fails | Appends stderr to the result the agent sees |
| `Stop` | When the agent finishes a turn | Continues the turn once with stderr as the instruction |

Tool events take an optional **tool matcher**: a tool name or anchored regex
such as `shell|write|edit` or `mcp:.*`. Empty or `*` matches every tool. Tool
events only fire for the built-in agent; ACP agents run their own tools, so
they only see `UserPromptSubmit` and `Stop`.

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

`UserPromptSubmit` adds `prompt`; `PostToolUse`/`PostToolUseFailure` add
`tool_response: { success, content }`. Exit 0 continues; exit 2 blocks as in the
table; any other exit code, a timeout or a launch failure shows a status-line
notice and the turn continues.
