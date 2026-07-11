# Model configuration

wisp-science calls remote LLM APIs through model profiles. Desktop users
configure these in **Settings -> Models**. Each row is a model profile with its
own display name, provider, API URL, model ID, advanced options, and API key.

Model profiles describe model access and capabilities for the **built-in Wisp
agent**. External coding agents (Codex / Claude via ACP) are configured
separately — see [ACP Agents](acp-agents.md). Do not put an ACP launch command
in a Models profile.

For image workflows, mark an API profile as **Supports image input** and optionally **Use for image analysis**. `view_image` and image reads call that assigned vision model and return text observations to the main agent, so the active/default chat model can remain non-visual.

## API providers

| Provider | Use when | Required fields |
| --- | --- | --- |
| OpenAI-compatible | DeepSeek, GLM, local gateways, or any `/chat/completions` compatible endpoint | API URL, Model ID, API key |
| OpenAI (Responses API) | OpenAI reasoning/tool-call models through `/v1/responses` | API URL, Model ID, API key |
| Anthropic | Claude API through `/v1/messages` | API URL, Model ID, API key |

API keys are stored in the OS keyring. They are not stored in SQLite.

The desktop app stores model profile metadata in `.wisp/wisp.sqlite`. Existing single-model installs are migrated into a `default` model profile the first time settings are loaded.

## Headless CLI

The `wisp-science` headless CLI uses environment variables and supports API providers:

```powershell
$env:WISP_PROVIDER = "openai"           # openai, openai_responses, or anthropic
$env:WISP_API_URL  = "https://api.deepseek.com"
$env:WISP_MODEL    = "deepseek-v4-pro"
$env:WISP_API_KEY  = "<your provider key>"
cargo run -p wisp-cli
```
