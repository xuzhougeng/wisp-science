# MCP long calls and connection recovery

## User-visible behavior

MCP `tools/call` has **no default execution deadline** in the native Agent, MCP
Apps, or the ACP bridge, over stdio or Streamable HTTP. This removes the former
120-second transport ceiling and 30-second App ceiling. Connection setup,
initialization, discovery, resource reads and orderly shutdown still have
technical deadlines. App argument/result limits, approval policy, visibility,
Plan mode, concurrency limits and iframe isolation remain in force. Wisp does
not override timeouts implemented inside a plugin or its upstream API/CLI.

Stopping a conversation or closing an App cancels its wait, not its shared MCP
server. The Host sends a best-effort `notifications/cancelled` for a sent request.
Servers may ignore cancellation. **Cancellation is not confirmation that a
GitHub write, publication, or other external operation stopped or rolled back.**
For stdio, a dedicated writer finishes any partially written JSON-RPC frame;
queued, unsent cancelled requests are skipped. Late responses are discarded.

Wisp owns connections separately from transient Agent instances and views.
Scopes include project, conversation, state scope, execution context and
connector configuration. Normal connections live until Wisp exits or the user
disables, replaces or explicitly restarts them, or deletes their conversation.
Deleting a conversation closes its connections, interrupts initialization and
rejects late background restore attempts; other conversations keep their own
connections. Failed initialization, broken
pipes and exited subprocesses are still cleaned up. HTTP session shutdown does
not terminate the remote service.

## Recovery and historical Apps

- A disconnected connector reconnects on the next use. Concurrent recovery
  attempts are coalesced. Failure is reported; there is no endless retry loop.
- Opening a saved conversation reconnects its currently enabled plugins. No
  previously unfinished tool request is resumed or replayed.
- A lost response produces an **unknown outcome**. Check the external state or
  plugin receipt before deciding whether to repeat a write. SFL plans cached in
  the server process may need to be generated again after a restart.
- Historical Apps with a Host-authored binding can obtain a new bridge and a
  fresh resource document. Their displayed results remain historical. Old
  iframe callbacks cannot acquire the replacement bridge.
- Older saved Apps without a reliable binding remain historical displays.
  Reopen them through the plugin to establish a connection; Wisp does not guess
  the server from a tool name.
- The native App controls include **Reconnect plugins / 重新连接插件**. This
  explicitly restarts the current conversation's connectors after warning about
  potentially committed external operations. It never repeats a tool call.

## Host/bridge contract and diagnostics

The ACP bridge proxies remote MCP calls through a private loopback endpoint.
Each launch has a memory-only capability bound to its project, conversation and
connector grant. It is passed only in that bridge's launch environment, never
persisted in SQLite or forwarded to plugin environments. Browser-origin
requests are rejected. A streaming lease cancels that bridge's pending requests
when the bridge disappears; the Host retains plugin processes. JSON-RPC stdout
never carries diagnostics.

Local bridge tools (including skills, memory, Run queries/cancellation and
questions) do not initialize unrelated plugins or wait for remote discovery.
Remote tool discovery and calls share a separately initialized catalog, so a
slow plugin cannot block those local requests.

Host tracing records `mcp.connection.*` and `mcp.request.*` events, with logical
identity, connection generation, request ID, method, tool, elapsed time and
outcome. In Windows release builds these use the existing `wisp.log` sink.
Arguments, result bodies, credentials, launch environments and configuration
descriptors are not audit fields. The bridge must not rotate/open the main log
using the desktop startup log initializer.

## Verification

Automated tests use fake subprocesses and loopback HTTP servers, not a real
SFL library or GitHub Apply. Coverage includes virtual-time long calls,
mid-write cancellation, process-tree cleanup, SSE completion before EOF,
session loss, no automatic write replay, scope isolation, capability revocation,
and historical App/reconnect controls.

Manual acceptance on Windows: run a known-safe long read, stop its wait and
confirm a subsequent status query succeeds; then restart Wisp, open the same
conversation, and verify plugin calls and a freshly bound App. Real external
writes require checking their receipts/results separately and must not be used
as an unattended retry test.

This change does not repair previously partial SFL/GitHub operations, alter the
Library or its locks, persist live RPCs/PIDs, or change non-MCP Run cancellation.
