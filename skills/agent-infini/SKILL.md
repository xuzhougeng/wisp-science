---
name: agent-infini
description: Use the InfiniSynapse CLI (`agent_infini`) for multi-turn AI data-analysis tasks, database/RAG context, and task workspace files. Use when the user mentions InfiniSynapse, agent_infini, database or RAG analysis, or asks to delegate analysis through InfiniSynapse.
license: Apache-2.0
---

# agent_infini

`agent_infini` talks to the InfiniSynapse backend REST API. Use it for database-backed analysis, RAG-backed analysis, multi-turn task conversations, and task workspace files.

Default binary locations:

- Linux/macOS: `~/.infini/bin/agent_infini`
- Windows: `%USERPROFILE%\.infini\bin\agent_infini.exe`

If `agent_infini` is not on `PATH`, call the full path.

## Credential setup

wisp-science stores the InfiniSynapse API key in Settings -> Credentials and initializes the CLI with:

```bash
agent_infini init --api-key "sk-xxx"
```

If the CLI is missing, install it:

```powershell
irm https://infinisynapse.cn/cli-install/install.ps1 | iex
```

```bash
curl -fsSL https://infinisynapse.cn/cli-install/install.sh | bash
```

Do not print API keys or dump config files. If authentication fails, ask the user to update the InfiniSynapse API key in Settings -> Credentials or rerun `agent_infini init`.

## Recommended workflow

1. Check the CLI: `agent_infini version`
2. List resources: `agent_infini db ls` and `agent_infini rag ls`
3. Check enabled context: `agent_infini task context`
4. Enable needed context: `agent_infini db enable <id>` or `agent_infini rag enable <id>`
5. Start a task: `agent_infini task new "..."`
6. Continue the same task: `agent_infini task ask <taskId> "..."`
7. Inspect outputs: `agent_infini task file <taskId>`, `preview`, or `download`

## Commands

```bash
agent_infini task new "Analyze user growth trend"
agent_infini task ask <taskId> "Show it as a bar chart"
agent_infini task ls [--page N] [--page-size N] [--search Q]
agent_infini task show <taskId>
agent_infini task context
agent_infini task cancel <taskId>
agent_infini task rm <id1> [id2 ...]
agent_infini task file <taskId>
agent_infini task preview <taskId> <fileName>
agent_infini task download <taskId> <fileName> [-o dir]
```

```bash
agent_infini db ls [--name N] [--type T] [--enabled] [--disabled]
agent_infini db enable <id> [id...]
agent_infini db disable <id> [id...]
```

```bash
agent_infini rag ls [--keyword K] [--enabled] [--disabled]
agent_infini rag enable <id> [id...]
agent_infini rag disable <id> [id...]
```

Useful global flags:

- `--json`: force JSON output
- `--table`: force table output for list commands
- `--api-key <key>`: override configured API key for this call
- `--server <url>`: override server address
- `--console <url>`: override Console API URL
- `--prefer-language <lang>`: `en`, `zh_CN`, `ar`, `ja`, `ko`, `ru`

## Output and errors

Default output is JSON:

```json
{"success": true, "data": {}}
{"success": false, "error": "error message"}
```

Common fixes:

- Token expired: update Settings -> Credentials or rerun `agent_infini init`
- Server unreachable: check network and `--server`
- Task not found: run `agent_infini task ls`
- No enabled resources: run `agent_infini task context`, then enable DBs/RAGs
