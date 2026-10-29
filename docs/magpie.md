# Magpie model connection

The optional [Magpie Agent plugin](https://github.com/xuzhougeng/magpie-agents)
lets Magpie select the model for Wisp Science's built-in HTTP agent. It requires
a build containing `wisp_llm::magpie`; older released applications do not read
the connection file.

## Connect and disconnect

Configure a provider in Magpie, then install the plugin from its source checkout:

```sh
magpie plugin add /absolute/path/to/magpie-agents/packages/agent-wisp-science
magpie wisp-science magpie/provider/model
```

Replace `provider/model` with an available model and keep the gateway running.
After npm publication, use
`magpie plugin add @xuzhougeng/magpie-agent-wisp-science@alpha` instead.

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
`baseURL`, `apiKey`, and `model`; all three must be valid for the override to
activate. Missing, partial, invalid, oversized, or `{}` files use the normal
personal configuration. The reader does not write this file or copy its key
into the database/keyring.

The override uses OpenAI Chat Completions for built-in HTTP model calls. Personal
profiles remain editable, and their validation tests the fields in the form.
Model changes in the personal editor take effect after Magpie is disconnected.
External ACP agents and dedicated image/video generation configurations retain
their own model management. A model must support any requested tools or images;
the integration file does not declare capabilities.

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
