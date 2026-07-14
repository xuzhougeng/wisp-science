# ACP Client Integration and Vendor CLI Retirement Plan

> **Status:** Implemented in this change; this document now serves as the
> architecture and verification plan for the shipped ACP v1 integration.
>
> **Date:** 2026-07-11
>
> **Scope:** Add Wisp as an ACP v1 client, then retire the Codex App Server,
> `codex exec`, Codex-as-tool, and Claude Code JSONL integrations.

## Goal

Make external coding agents a protocol-level backend rather than a collection
of vendor-specific providers and tools:

- Wisp is the **ACP Client**.
- Codex, Claude, Gemini, or another implementation is an **ACP Agent** launched
  as a local child process.
- Wisp speaks only stable ACP v1 over stdio. Vendor adapters live outside this
  repository.
- Wisp's existing direct HTTP model path remains available for its built-in
  `wisp-core::Agent`.
- Existing MCP connections and the Wisp scientific MCP bridge remain separate
  from ACP and are reused through ACP's `mcpServers` session field.

The intended result is a net deletion: one small protocol client replaces more
than 13,000 lines of Codex/Claude command discovery, private JSON-RPC/JSONL,
configuration mirroring, and fallback behavior.

## Decisions

1. **ACP is a conversation backend, not a `wisp_llm::Provider` and not a Wisp
   tool.** An ACP Agent owns its session, model loop, tools, and permissions.
   The existing provider trait represents one model completion and cannot model
   the ACP lifecycle without rebuilding the protocol incorrectly.
2. **Use the official Rust SDK.** Do not hand-write another JSON-RPC client and
   do not depend directly on the lower-level schema crate.
3. **Keep ACP agent profiles separate from HTTP model profiles.** A command
   that starts an agent is not an API model configuration.
4. **First release: local stdio only.** Streamable HTTP remains a draft; WSL,
   SSH, agent registry installation, and remote transports are later work.
5. **Advertise only implemented client capabilities.** The first usable slice
   does not advertise client filesystem or terminal methods. ACP agents may use
   their own filesystem and terminal implementation in the project `cwd`.
6. **One ACP process per active Wisp frame.** This is the smallest lifecycle
   that preserves frame isolation. Do not add a cross-session process pool
   until measured process cost requires it.
7. **Remove both private CLI integrations.** There is no Claude Code
   compatibility island after cutover; Claude is used through an ACP Agent or
   through the retained Anthropic HTTP provider.

## Current Code Findings

### Why the current integration should be replaced

- `src-tauri/src/codex_app_server.rs` is a 3,591-line client for Codex's private
  `app-server --stdio` protocol.
- `src-tauri/src/codex_provider.rs` is a 5,341-line provider/runtime manager for
  Codex threads, turns, configuration snapshots, Plan mode, permissions, and
  event conversion.
- `src-tauri/src/codex_runtime.rs` is a 2,337-line CODEX_HOME discovery,
  mirroring, rewriting, and MCP injection layer.
- `src-tauri/src/codex_tool.rs` is a 388-line `codex exec --json` tool wrapper.
- `src-tauri/src/local_runner.rs` is a 2,285-line mixed Codex/Claude command,
  JSONL, WSL, prompt, and private session compatibility layer.
- `src-tauri/src/lib.rs:2253-2957` selects Codex App Server, falls back to
  `codex exec`, or launches Claude Code before reaching the normal Wisp Agent.
- `src-tauri/src/models.rs:37-83` stores CLI executable, sandbox, Plan model,
  and other Codex-only fields inside `ModelProfile`.
- `ui/src/dto.rs:388-637` and large sections of `ui/src/main.rs` model Codex
  runtime snapshots and configuration generations directly in the frontend.

These are multiple implementations of the role ACP standardizes.

### Boundaries worth keeping

- `crates/wisp-llm/` and `crates/wisp-core/` remain the built-in, direct HTTP
  agent backend.
- `crates/wisp-tools/`, `crates/wisp-runtime/`, and `crates/wisp-skills/` remain
  Wisp-native capabilities.
- `crates/wisp-mcp/` remains Wisp's MCP client implementation.
- The stdio `BridgeServer` in `src-tauri/src/mcp_bridge.rs` remains useful. It
  exposes Wisp skills, scientific MCP tools, custom MCP connections, and Run
  Manager tools to any ACP Agent.
- `src-tauri/src/mcp_bridge.rs:442-537` (`WispToolRouter`) is Codex App Server
  dynamic-tool glue and can be removed.
- The normal `openai`, `openai_responses`, and `anthropic` HTTP providers are
  independent of the retired CLI backends and must remain.

## Protocol Baseline

This plan targets stable ACP wire `protocolVersion = 1` as published on
2026-07-11.

The required usable flow is:

```text
initialize
  -> authenticate (only when the selected agent requires it)
  -> session/new
  -> session/prompt
       <- session/update ...
       <- session/request_permission ...
  -> session/cancel (when the user stops the turn)
  <- session/prompt response with stopReason
```

Reconnection is capability-gated:

1. Prefer `session/resume` when advertised; it restores agent state without
   replaying history.
2. Otherwise use `session/load` when `loadSession` is advertised; suppress
   replay notifications from being appended to Wisp's already persisted
   transcript.
3. If neither exists, the saved Wisp transcript remains readable, but the ACP
   session cannot continue after its process exits.

Only stdio is in scope. ACP messages are newline-delimited UTF-8 JSON-RPC 2.0;
stdout is protocol-only and stderr is diagnostic output.

### Stable capabilities in the first useful slice

| Area | Initial behavior |
| --- | --- |
| Protocol | Negotiate ACP v1 and reject a mismatched version clearly. |
| Authentication | Display stable agent-managed `authMethods` and call `authenticate(methodId)`. Do not store credentials in SQLite. |
| Prompts | Support `Text` and `ResourceLink`, the baseline content blocks. |
| Session updates | Handle messages, thoughts, tools, plans, commands, config/mode, session info, and usage without panics. |
| Permissions | Implement `session/request_permission` with the exact options supplied by the Agent. |
| Cancellation | Send `session/cancel`, cancel pending permission requests, drain final updates, then force-kill only after a grace timeout. |
| MCP | Pass the existing Wisp stdio MCP bridge in `session/new` and reconnect requests. |
| Session config | Prefer generic `configOptions`; support legacy ACP modes only as a fallback. |
| Client filesystem | Do not advertise in v1. |
| Client terminal | Do not advertise in v1. |
| Optional session methods | Use resume/load/close only after checking the advertised capability. |

## Target Architecture

```text
Leptos UI
   |
   | send_message(frame, backend, prompt)
   v
Tauri backend router
   |                              |
   | internal HTTP model          | ACP agent profile
   v                              v
wisp-core::Agent             crates/wisp-acp
   |                              |
   v                              | stdio ACP v1
wisp-llm provider                 v
                            external ACP Agent
                                  |
                                  | session/new(mcpServers=[wisp_bridge])
                                  v
                            existing Wisp MCP bridge

ACP updates/permissions -> Tauri generic events -> existing chat/event surface
```

### New crate: `crates/wisp-acp`

This crate is the protocol and process boundary. It should initially stay
small; split files only when the process actor and protocol mappings require a
real test boundary.

Owned responsibilities:

- Launch one configured executable with an argument vector, never through a
  shell command string.
- Own stdin/stdout/stderr and the official ACP connection.
- Perform initialize/auth/session/prompt/config/cancel/close operations.
- Convert official SDK callbacks into a small Wisp-neutral event enum.
- Correlate permission requests and return the selected protocol option.
- Bound stderr memory and guarantee child cleanup on handle drop.

Proposed public types:

```text
AcpAgentProfile { id, label, command, args }
AcpAgentInfo { protocol_version, implementation, capabilities, auth_methods }
AcpSessionHandle
AcpSessionEvent
AcpPermissionRequest
AcpPromptOutcome { stop_reason }
```

Do not add environment values to `AcpAgentProfile` in v1. Agent-managed login
or the OS environment covers the first release without putting secrets in the
SQLite settings JSON. Add keyring-backed environment references only when a
real Agent requires them.

### Tauri integration: `src-tauri/src/acp.rs`

This module owns application semantics, not JSON-RPC:

- CRUD and validation commands for ACP agent profiles.
- Wisp frame to ACP session binding.
- Construction of the Wisp MCP server descriptor for `session/new`.
- Mapping `AcpSessionEvent` to generic `AgentEvent` values.
- Transcript persistence and UI permission routing.
- Cleanup on stop, frame deletion, project deletion, and app exit.

`send_message` remains the single frontend command. Add a small backend
dispatch before the existing internal Agent path:

```text
existing frame has ACP binding -> run ACP turn with the bound profile
new empty frame + acp_agent_id -> create binding and run ACP turn
otherwise -> existing wisp-core Agent path
```

An ACP Agent may only be attached to an empty frame. Do not pretend that an
existing Wisp or legacy CLI transcript can be injected as native ACP history.

### Runtime ownership

Replace the vendor-specific fields in `SessionRuntime` with an explicit
backend runtime:

```text
SessionRuntime
  workflow lock
  cancel state
  persisted sequence
  backend runtime:
    - internal Agent, or
    - ACP session handle
```

The ACP actor owns non-`Send` SDK details if the selected SDK API requires a
local executor. Tauri receives a `Send` command/event handle; SDK constraints
must not leak into UI code.

Do not share an ACP connection across frames in v1. A later pool may key by
`(project_id, acp_agent_profile_id)` if process count becomes a measured
problem.

## Persistence

### ACP agent profiles

Store non-secret profiles under a new `acp_agent_profiles` settings key, using
the existing `model_profiles` CRUD style. This is configuration, not a new SQL
entity.

Validation rules:

- non-empty generated ID and label;
- non-empty executable path/name;
- argument vector preserved exactly;
- no shell splitting, interpolation, or implicit `npx` download;
- test connection launches, initializes, reports info/capabilities/auth, and
  then terminates cleanly.

### ACP session binding

Add an idempotent `0007_acp_sessions` migration and a small store module.

```text
acp_sessions
  frame_id             TEXT PRIMARY KEY REFERENCES frames(id) ON DELETE CASCADE
  agent_profile_id     TEXT NOT NULL
  profile_fingerprint  TEXT NOT NULL
  agent_session_id     TEXT NOT NULL
  cwd                  TEXT NOT NULL
  protocol_version     INTEGER NOT NULL
  agent_info_json      TEXT NOT NULL DEFAULT '{}'
  capabilities_json    TEXT NOT NULL DEFAULT '{}'
  created_at           INTEGER NOT NULL
  updated_at           INTEGER NOT NULL

UNIQUE(agent_profile_id, agent_session_id)
```

The binding is written immediately after `session/new` succeeds and before the
first prompt. Frame ownership and the current project root must be validated
before every prompt or reconnect.

`profile_fingerprint` covers the executable, argument vector, and other
non-secret launch configuration. Editing a profile must never silently resume
an old session through a different command. A fingerprint or `cwd` mismatch
makes the old frame readable but requires a new ACP frame to continue.

`agent_info_json` and `capabilities_json` are diagnostic snapshots only. Every
new process must run `initialize` again and treat that response, not SQLite, as
the capability authority.

Do not store credentials, access tokens, the Agent's private config, or copies
of SSH keys in this table.

### Transcript policy

- Persist the accepted user message using the existing `messages` table.
- Aggregate `agent_message_chunk` updates into the assistant message and stamp
  `model_name` with the ACP Agent/profile label, not a guessed model ID.
- Aggregate displayable `agent_thought_chunk` text into the existing assistant
  `Message.reasoning` field. Tool updates, plans, and permissions remain live
  structured events in v1. Do not introduce a general event-sourcing table
  solely for ACP.
- Existing export and manual review continue to operate on the plain Wisp
  transcript.
- Rich tool/plan replay after app restart is an explicit later feature.

## Event and UI Mapping

Do not reuse Codex DTOs with renamed fields. Add the smallest generic shapes
needed by both ACP and the current UI.

| ACP update | Wisp mapping |
| --- | --- |
| `user_message_chunk` | Used only for `session/load` replay reconciliation; do not duplicate locally persisted user messages. |
| `agent_message_chunk` | `AgentEvent::Text`; aggregate for assistant persistence. |
| `agent_thought_chunk` | `AgentEvent::Reasoning`; also accumulate into the final assistant message's existing reasoning field. |
| `tool_call` / `tool_call_update` | New ID-addressed generic tool update containing call ID, title, kind, status, content, and locations. |
| `plan` | New generic plan event containing the full entry list; replace the displayed list on every update. |
| `available_commands_update` | Update per-session slash-command state; command palette exposure may follow later. |
| `current_mode_update` | Update the legacy mode fallback only. |
| `config_option_update` | Update generic composer session configuration. |
| `session_info_update` | Update the frame title only when it has not been manually renamed. |
| `usage_update` | Map available fields to the existing usage surface; do not invent missing context values. |
| Unknown future update | Ignore safely and log at debug level. |

The existing `ToolCall`/`ToolResult` events may remain for the internal Agent.
ACP tool updates need a call ID because multiple tool calls can overlap and
cannot be matched reliably by title.

Extend the terminal turn event with an optional protocol stop reason so
`end_turn`, `max_tokens`, `max_turn_requests`, `refusal`, and `cancelled` are
not collapsed into an undifferentiated `Done`.

### Permission requests

Do not force ACP permission requests through the current frame-keyed boolean
`ConfirmMap`; it cannot represent concurrent requests or Agent-provided option
IDs.

Add:

- a `permission-request` frontend event with a unique request ID, frame ID,
  tool call summary, and the exact ACP options;
- a `respond_acp_permission(request_id, option_id | null)` command;
- a pending map keyed by request ID;
- frame-level `awaiting_confirm` bookkeeping for the projects dashboard.

The UI may reuse the existing inline approval card styling, but it must render
the Agent's `allow_once`, `allow_always`, `reject_once`, and `reject_always`
choices rather than translating them into Wisp's unrelated project/global
approval grants.

When a prompt is cancelled or a frame is deleted, resolve every pending ACP
permission request for that frame as `cancelled` before closing the session.

### Session configuration

Replace the hard-coded Codex model/effort/Plan controls with generic ACP
`configOptions`:

- render the option label, description, category, current value, and options
  sent by the Agent;
- call the ACP set-config method with opaque IDs and values;
- never manufacture model, effort, mode, or sandbox choices;
- use deprecated ACP modes only when the Agent does not provide config options;
- keep config state scoped to the bound ACP frame.

ACP Plan notifications are progress display. They do not imply Wisp's old
Codex-specific “approve this Plan, then start another turn” workflow.

## MCP Reuse

Keep the stdio bridge entrypoint in `src-tauri/src/main.rs` and pass it to the
Agent as a standard ACP `McpServer::Stdio` descriptor.

Refactor `src-tauri/src/mcp_bridge.rs` only where the old providers leak in:

- rename comments from “local runners (Codex CLI / Claude Code)” to “external
  ACP agents”;
- keep `BridgeServer`, its CLI parsing, skill filtering, scientific tools,
  custom MCP proxying, and Run Manager tools;
- delete `WispToolRouter`, which only served Codex App Server dynamic tools;
- delete Codex/Claude `plan_safe` assumptions. ACP config option IDs are opaque,
  so Wisp cannot infer read-only policy from a mode name;
- retain the existing hard failure for dangerous non-interactive Run commands.

ACP does not replace MCP: ACP owns the editor/client-to-agent conversation;
MCP gives the selected Agent access to Wisp's scientific tools.

## Retirement Matrix

### Delete completely after ACP parity

- `src-tauri/src/codex_tool.rs`
- `src-tauri/src/codex_app_server.rs`
- `src-tauri/src/codex_provider.rs`
- `src-tauri/src/codex_runtime.rs`
- `src-tauri/src/local_runner.rs`
- module declarations, `AppState.codex`, `SessionRuntime.local_child`, Codex
  interrupt/drop logic, private CLI routing, and Codex Tauri commands in
  `src-tauri/src/lib.rs`
- local-runner validation branches in `src-tauri/src/settings_commands.rs`
- Codex/Claude runner fields and provider normalization in
  `src-tauri/src/models.rs`
- Codex runtime/config/audit DTOs and callbacks in `ui/src/dto.rs`,
  `ui/src/main.rs`, and `ui/src/settings_view.rs`
- Codex/Claude provider choices, private Plan controls, styles, translations,
  Playwright cases, and mock bridge state
- `toml = "0.8"` from `src-tauri/Cargo.toml` if the final reference check confirms
  no remaining direct use
- active Rust CRUD/types for `codex_turn_configs` and the provider-specific
  proposed-plan workflow

### Keep or generalize

- direct HTTP `ModelProfile` and keyring behavior
- `wisp-core::Agent`, Wisp tools, skills, Python, reviews, store, and Run Manager
- `crates/wisp-mcp`
- stdio Wisp MCP bridge, minus Codex dynamic-tool and Plan-mode branches
- generic chat text, reasoning, tool-card, usage, error, and done UI
- generic plan-card visuals if useful for ACP's structured plan entries; replace
  the Codex plan IDs/revisions/actions rather than preserving their semantics
- historical session messages, exports, artifacts, and execution logs

### Do not delete by string match

- historical release notes
- `.codex/` development prompts or user files
- documentation where “Codex” means the implementation worker rather than a
  product provider
- generated-by attribution in old plans

## Compatibility and Data Migration

1. Do not automatically convert `ModelProfile(provider = "codex_cli")` or
   `claude_code` into an ACP profile. `codex` and `claude -p` do not speak ACP.
2. Before filtering retired profiles, copy their raw non-secret JSON to a
   `retired_local_runner_profiles_v1` setting for recovery. If the active model
   is retired, select the first valid HTTP profile and show a one-time migration
   notice directing the user to add an ACP Agent.
3. Do not delete `.wisp/codex-home`, global Codex/Claude config, or Agent-owned
   session history automatically. Cleanup is documented and user-controlled.
4. Old Wisp transcripts remain readable and exportable. Old Codex/Claude
   external thread IDs are not ACP session IDs and cannot be resumed through
   ACP. Starting an ACP turn requires a new empty Wisp frame.
5. Keep already shipped migration versions and legacy SQL tables intact. Stop
   writing them, remove unused runtime CRUD, and add the new ACP table
   additively. Do not `DROP` legacy tables in this migration.
6. Deleting a Wisp session calls capability-gated `session/close` and terminates
   its local process. It must not call destructive `session/delete` without a
   separate explicit user action.
7. Branching a frame copies messages but never copies an ACP binding. Rewind or
   message editing is rejected for ACP frames in v1; the backend guard remains
   even if an older frontend calls the command directly.

## Delivery Plan

### PR 0: SDK/MSRV compatibility gate

**Purpose:** Prove that the official SDK fits the workspace before product code
depends on it.

Changes:

- Raise workspace `rust-version` from 1.80 to 1.88; keep Wisp crates on edition
  2021.
- Update README prerequisites and add an explicit Rust 1.88 CI check rather
  than relying only on floating `stable`.
- Add a minimal `crates/wisp-acp` workspace member with an exact
  `agent-client-protocol = "=1.2.0"` dependency and no unstable features.
- Compile a minimal in-memory client/agent handshake.

Acceptance:

- The official SDK compiles on Linux, macOS, Windows, and the workspace's
  minimum Rust version.
- No `unstable` umbrella, protocol v2, schema crate, or hand-written JSON-RPC is
  introduced.
- `cargo test --workspace` still passes.

If Rust 1.88 cannot be adopted, stop here and revisit the product requirement;
do not create a second private ACP implementation to preserve Rust 1.80.

### PR 1: ACP process client and fake Agent

**Purpose:** Implement the protocol lifecycle without touching Tauri or UI.

Changes:

- Implement process launch, initialization, stable authentication, new session,
  prompt/update streaming, permission response, cancellation, config changes,
  resume/load/close, stderr capture, and shutdown in `crates/wisp-acp`.
- Add an in-process or re-exec fake ACP Agent. It must not require an installed
  Codex, Claude, network, API key, or shell.

Acceptance:

- Tests cover protocol mismatch, capability omission, auth-required flow,
  prompt updates, concurrent permission IDs, cancellation tail updates,
  child exit, stderr bounds, and cleanup.
- Unknown stable/future update variants do not crash the client.
- Commands and args round-trip without shell parsing on all platforms.

### PR 2: Profiles and durable session binding

**Purpose:** Add one durable ACP abstraction before routing real conversations.

Changes:

- Add `acp_agent_profiles` CRUD and connection-test/auth commands in
  `src-tauri/src/acp.rs`.
- Add `0007_acp_sessions`, `crates/wisp-store/src/acp_sessions.rs`, and store
  round-trip APIs.
- Add the ACP handle variant to `SessionRuntime` and cleanup hooks for stop,
  session delete, project delete, and app exit.
- Keep all existing Codex/Claude paths operational in this PR.

Acceptance:

- Profile validation rejects empty commands and preserves argument boundaries.
- Session bindings enforce frame/project ownership and reject attaching ACP to
  a non-empty frame.
- Profile fingerprint or project `cwd` changes cannot silently reuse an old ACP
  session.
- Store migrations are idempotent on a new DB and on a pre-ACP DB.
- Deleting a frame cascades its ACP binding without dropping legacy data.

### PR 3: End-to-end ACP turn and Wisp MCP bridge

**Purpose:** Make one ACP Agent usable from the existing chat command.

Changes:

- Add the small `send_message` backend dispatch and `run_acp_turn` path.
- Pass text, file `ResourceLink`s, and the Wisp MCP bridge to `session/new`.
- Map ACP events, persist user/final assistant text, and implement permission
  request/response.
- Implement stop/cancel/graceful-kill behavior and capability-gated reconnect.
- Remove `WispToolRouter` and vendor-specific `plan_safe` behavior from the MCP
  bridge, but retain its stdio server.

Acceptance:

- Fake Agent smoke: initialize -> new -> prompt -> tool/plan/text updates ->
  permission -> completed.
- Stop affects only the requested frame and leaves parallel frames running.
- A reconnect uses resume, then load, then a clear non-resumable state according
  to capabilities.
- No ACP request can bind a frame from another project.
- Direct HTTP Agent tests remain unchanged and passing.

### PR 4: Generic ACP settings and composer UI

**Purpose:** Replace vendor UI with protocol-discovered state.

Changes:

- Add an Agents settings page for command/args, Test Connection, agent info,
  auth methods, and delete.
- Group the composer selector into Wisp Models and ACP Agents without adding ACP
  as a fake model provider.
- Lock the ACP Agent selection after the first prompt.
- Render generic config options/mode fallback, ID-addressed tool updates,
  structured plans, permission choices, usage, and session status.
- Hide edit/rewind and empty “Resume” actions for ACP frames in v1; ACP has no
  stable client-side rewind operation.

Acceptance:

- Playwright mocks cover agent creation/test/auth, selecting an ACP Agent,
  streaming a turn, config updates, overlapping tool calls, permission choice,
  cancellation, project switching, and a non-resumable saved session.
- Unsupported prompt content is disabled or sent as a baseline ResourceLink;
  the UI never sends image/audio/embedded blocks without capability support.
- Existing HTTP models and Specialists UI continue to work. Specialists remain
  an internal Wisp Agent feature in this release.

### PR 5: Remove private Codex and Claude integrations

**Purpose:** Complete the cutover only after ACP parity is tested.

Changes:

- Delete the five vendor runtime files listed in the retirement matrix.
- Remove their Tauri commands, state, routing, model fields, UI DTOs, settings,
  runtime snapshots, Plan approval flow, audit tables' active APIs, tests,
  styles, and translations.
- Add the retired-profile backup/filter behavior and one-time notice.
- Replace README's “Local Codex runtime and Plan mode” section with ACP Agent
  setup and limitations. Keep historical release notes unchanged.
- Run a final reference/dependency prune; remove `toml` if unused.

Acceptance:

- `rg` finds no production references to `codex_app_server`, `codex_provider`,
  `codex_runtime`, `codex_tool`, `local_runner`, `codex_cli`, or `claude_code`.
- A retired profile can never fall through to the HTTP provider builder.
- Historical transcripts and exports still load.
- ACP Codex and ACP Claude use the same Wisp code path and differ only by their
  configured executable/arguments and negotiated capabilities.

## Verification Matrix

Run narrow tests first in every PR, followed by the repository-required suite.

```bash
cargo fmt --all -- --check
cargo test -p wisp-acp
cargo test -p wisp-store acp
cargo test -p wisp-tauri acp
cargo test --workspace
cd ui && cargo check --target wasm32-unknown-unknown
cd ../ui-tests && npm ci && npx playwright test
```

ACP tests must use a fake Agent and temporary directories. They must never
require a real ACP adapter, Codex/Claude installation, login, API key, network,
WSL distro, SSH host, or GPU.

Manual smoke, after the automated fake-Agent path passes:

1. Configure one already-installed ACP Agent command.
2. Test connection and complete agent-managed authentication if advertised.
3. Create a new Wisp session, select the ACP Agent, and send a text prompt.
4. Verify text, plan, tool, permission, config, and stop behavior.
5. Restart Wisp and verify resume/load or the explicit non-resumable message.
6. Run the same steps with two different ACP Agents to prove there is no vendor
   branch in Wisp.

## Known Initial Limitations

- Local stdio only; no Streamable HTTP, WSL-hosted, SSH-hosted, or scheduler
  ACP transport.
- No ACP Registry browse/install/update UI. Users configure an installed Agent
  command manually.
- No client-provided filesystem or terminal capability. Add them only with
  project-root containment, output limits, process cleanup, and platform tests.
- No image/audio/embedded prompt blocks until the matching negotiated capability
  and existing attachment UX are wired end to end.
- No ACP session fork/rewind, unstable elicitation, MCP-over-ACP, proxy chain, or
  protocol v2 features.
- No rich tool/plan event replay from SQLite in the first release.
- No process pool; an active ACP frame owns its Agent process.
- ACP permission requests improve UX but are not a sandbox. The configured
  local Agent process has the operating-system access granted to Wisp's user.

## Definition of Done

ACP support is complete for this plan when:

- Wisp can configure and launch an arbitrary local stdio ACP v1 Agent.
- Stable authentication, session creation, prompts, updates, permissions,
  cancellation, generic configuration, and capability-gated reconnect work.
- Wisp scientific capabilities reach the Agent through the retained MCP bridge.
- Direct HTTP Wisp Agent behavior remains intact.
- Codex and Claude private protocols, CLI JSONL parsers, runtime mirroring, and
  provider-specific frontend state are gone.
- No legacy profile silently becomes an HTTP request or an ACP command.
- All automated tests remain offline and cross-platform.

## Official References

- [ACP architecture](https://agentclientprotocol.com/get-started/architecture)
- [ACP v1 overview](https://agentclientprotocol.com/protocol/v1/overview)
- [Initialization and capabilities](https://agentclientprotocol.com/protocol/v1/initialization)
- [Authentication](https://agentclientprotocol.com/protocol/v1/authentication)
- [Session setup](https://agentclientprotocol.com/protocol/v1/session-setup)
- [Prompt turns](https://agentclientprotocol.com/protocol/v1/prompt-turn)
- [Tool calls and permissions](https://agentclientprotocol.com/protocol/v1/tool-calls)
- [Cancellation](https://agentclientprotocol.com/protocol/v1/cancellation)
- [Session config options](https://agentclientprotocol.com/protocol/v1/session-config-options)
- [Transports](https://agentclientprotocol.com/protocol/v1/transports)
- [Official Rust SDK](https://github.com/agentclientprotocol/rust-sdk)
- [ACP schema/versioning repository](https://github.com/agentclientprotocol/agent-client-protocol#versioning)
- [Official Codex ACP Agent](https://github.com/agentclientprotocol/codex-acp)
- [Official Claude Agent ACP adapter](https://github.com/agentclientprotocol/claude-agent-acp)

As of this plan, stable wire protocol is v1, the published schema release is
`v1.19.0`, and the official Rust SDK is `1.2.0`. Artifact/crate versions are not
wire versions; compatibility is determined by `initialize.protocolVersion` and
the negotiated capabilities.
