# Magpie model connection

The optional [Magpie Agent plugin](https://github.com/xuzhougeng/magpie-agents)
lets Magpie select the model for Wisp Science's built-in HTTP agent. It requires
a build containing `wisp_llm::magpie`; older released applications do not read
the connection file.

## Connect and disconnect

Configure a provider in Magpie, then install the plugin:

```sh
magpie plugin add @hoptoper/magpie-agent-wisp-science
magpie wisp-science magpie/provider/model
```

Replace `provider/model` with an available model and keep the gateway running.
To try an unreleased plugin, pass its source checkout instead:
`magpie plugin add /absolute/path/to/magpie-agents/packages/agent-wisp-science`.

Restart Wisp Science after every connection, model change, or disconnection.
The desktop and CLI read the connection once per process, so live agents and
their context limits use one consistent model. Runtime session labels show
`Magpie · <model>`. To disconnect, run `magpie wisp-science default` once and
restart the application. A newly launched CLI process also picks up the change.

接入、切换模型或断开后都需要重启 Wisp Science；原有个人模型和密钥会保留。

## File and scope

The fixed file is `~/.config/wisp-science/magpie.json`, or
`%USERPROFILE%\.config\wisp-science\magpie.json` on Windows. XDG_CONFIG_HOME,
APPDATA, and the native database directory do not relocate it. Magpie writes
`baseURL`, `apiKey`, and `model`, and newer plugins add `modelInfo`; the
three connection fields must be valid for the override to activate. Missing, partial, invalid, oversized, or `{}` files use the normal
personal configuration. The reader does not write this file or copy its key
into the database/keyring.

The override uses OpenAI Chat Completions for built-in HTTP model calls. Personal
profiles remain editable, and their validation tests the fields in the form.
Model changes in the personal editor take effect after Magpie is disconnected.
External ACP agents and dedicated image/video generation configurations retain
their own model management. A model must support any requested tools or images.

## Gateway sessions

Every model request through the Magpie connection carries an `X-Session-Id`
header with the conversation's ID: the desktop conversation (or delegated
child), or the CLI's `.wisp/session-id`. Wisp Science conversations then
appear on Magpie's Sessions page as gateway-backed, read-only sessions with
transcripts from the gateway's request ledger. Personal model profiles keep
their own session-header setting.

## Library: instructions, MCP servers and skills

Magpie's Library writes into three fixed Wisp Science user-level locations.
They are ordinary Wisp files (`~` is `%USERPROFILE%` on Windows) and work the
same without Magpie; Magpie keeps whatever else you put there.

| Library item | Location | Takes effect |
| --- | --- | --- |
| Instructions | `~/.wisp/AGENTS.md`; Magpie appends a delimited block and keeps your text | New conversations. An existing desktop conversation is marked stale and can use **Reload project rules…** |
| MCP servers | `~/.wisp/mcp.json` in the `mcpServers` format; Magpie merges in its servers | The next agent build: a new desktop conversation, or an existing one once its agent is rebuilt; the next CLI start |
| Skills | `~/.wisp/skills/<skill>`, usually a symlinked folder | **Settings → Skills → Reload skills** or a restart; the next CLI start |

User instructions have the lowest precedence: the project's `AGENTS.md` and
`.wisp/WISP.md` override them. MCP secrets stay `${NAME}` references in the
file. Wisp expands them from its own process environment when it starts a
server and skips a server whose variable is unset, so the desktop app needs the
variable in the environment it was launched from. Legacy SSE servers are skipped
with an error; stdio and Streamable HTTP (`"type": "http"`) are supported. See
[Skills](skills.md) and [basic configuration](basic-configuration.md#用户级-mcp-文件-wispmcpjson).

## Model facts and reasoning effort

Plugin builds that publish model facts also write
`modelInfo: { id, context, output, efforts }` with only the values Magpie knows.
Wisp Science uses the snapshot only when its `id` equals `model`; a stale or
malformed snapshot is ignored and the connection stays active.

| Value | Source, in order |
| --- | --- |
| Context capacity | `modelInfo.context`, the built-in catalog by exact model ID, 128,000 |
| Output limit (`max_tokens`) | `modelInfo.output`, the catalog, 8,192 |
| Reasoning levels | `modelInfo.efforts`, the catalog; otherwise none |

The composer's reasoning-effort menu lists only those levels while Magpie is
connected, and the choice is stored on the current conversation (a choice made
before the first message applies to the new conversation). Personal model
profiles are not changed. Default is medium, then high, then the first listed
level. A conversation's choice that the connected model does not list falls
back to that default instead of failing the request. When no levels are known,
no `reasoning_effort` is sent and the provider's own default applies.

The CLI applies `WISP_REASONING_EFFORT` the same way: a level the model lists is
sent, anything else uses the model default.

Restart after switching models in Magpie so the new snapshot is read.

Magpie restores the previous external fields on disconnect. An earlier valid
external file becomes active again; otherwise normal personal models resume.
On Magpie v0.1.1986, avoid two consecutive `default` commands: the second can
delete a restored model field. See the plugin verification report.

## Verification

The tests use temporary files and local mock servers; no provider account is
required:

```sh
cargo test -p wisp-llm magpie --offline
WISP_CATALOG_OFFLINE=1 cargo check -p wisp-cli -p wisp-tauri --offline
```

`crates/wisp-llm/examples/magpie_probe.rs` exercises the actual reader and provider
client against the official Magpie gateway using `MAGPIE_TEST_CONFIG`. See
`magpie-agents/docs/verification.md` for the coordinated three-application test.
