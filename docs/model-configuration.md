# Model configuration

wisp-science has two model paths:

- API providers: wisp calls a remote LLM API directly.
- Local runners: wisp launches an installed agent CLI and streams its JSON output back into the same chat UI.

Desktop users configure these in **Settings -> Models**. Each row is a model profile with its own display name, provider, model ID, advanced options, and API key when needed.

## API providers

| Provider | Use when | Required fields |
| --- | --- | --- |
| OpenAI-compatible | DeepSeek, GLM, local gateways, or any `/chat/completions` compatible endpoint | API URL, Model ID, API key |
| OpenAI (Responses API) | OpenAI reasoning/tool-call models through `/v1/responses` | API URL, Model ID, API key |
| Anthropic | Claude API through `/v1/messages` | API URL, Model ID, API key |

API keys are stored in the OS keyring. They are not stored in SQLite.

The desktop app stores model profile metadata in `.wisp/wisp.sqlite`. Existing single-model installs are migrated into a `default` model profile the first time settings are loaded.

## Local runners

Local runners do not use a wisp API key. They rely on the local CLI's own authentication and configuration.

| Provider | Command wisp runs | Key fields |
| --- | --- | --- |
| Codex CLI | `codex exec --json ...` | Model ID, Runner command, Codex profile, Runner sandbox, web search |
| Claude Code | `claude -p --output-format stream-json --verbose ...` | Model ID, Claude command |

Use `inherit` as the Model ID to keep the CLI's default model. Any other non-empty model ID is passed through as `--model`.

### Codex CLI

Install and log in to Codex before using this provider. Leave **Runner command** empty to use auto-detection, or set a full command such as:

```text
codex
C:\Users\you\AppData\Local\OpenAI\Codex\bin\...\codex.exe
wsl.exe -e codex
```

**Codex profile** is passed as `--profile`. **Runner sandbox** is passed as `--sandbox` and supports:

- `read-only`
- `workspace-write`
- `danger-full-access`

When **Enable Codex web search** is on, wisp passes `--search` to Codex.

Codex image attachments are passed as `--image` when the uploaded file has a supported image extension.

### Claude Code

Install and log in to Claude Code before using this provider. Leave **Claude command** empty to use `claude` from `PATH`, or set a full command/path if the desktop app cannot find it.

wisp runs Claude Code in non-interactive print mode and reads `stream-json` output. It uses `--permission-mode bypassPermissions`, so only use this provider in a workspace you trust.

## Platform notes

- macOS: if the app cannot find `codex` or `claude` when launched from Finder/Dock, set the full command path in the model profile.
- Windows: normal Windows workspaces run the configured command directly.
- Windows + WSL paths: WSL workspaces are routed through `wsl.exe -e codex` or `wsl.exe -e claude` by default.

## Headless CLI

The `wisp-science` headless CLI uses environment variables and supports API providers:

```powershell
$env:WISP_PROVIDER = "openai"           # openai, openai_responses, or anthropic
$env:WISP_API_URL  = "https://api.deepseek.com"
$env:WISP_MODEL    = "deepseek-v4-pro"
$env:WISP_API_KEY  = "<your provider key>"
cargo run -p wisp-cli
```

Local runner profiles are a desktop feature.
