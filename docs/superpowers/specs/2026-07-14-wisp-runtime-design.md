# Wisp Runtime: Persistent Local/Remote Python and R Sessions

**Date:** 2026-07-14
**Status:** Implemented (PR 1–5)
**Scope:** Rename `wisp-python` to `wisp-runtime` and evolve it into the
project-level manager for persistent exploratory Python and R sessions running in
local, WSL, or SSH execution contexts.

## 1. Decision summary

Terminated runtime cards offer **Dismiss / 移除**. Dismiss removes only the
selected dead instance from the in-memory registry, without launching an
interpreter or deleting project files. Live instances must first be stopped.
The current project's configured language slot remains available as Not started;
stale records from other projects disappear. Runtime generation counters survive
dismissal so guards cannot accidentally match a replacement. WSL worker deployment
uses scripts on stdin (the #1081 fix); both changes require an updated build and
are not present in the v1.8.1 installer.

wisp-science needs two different execution planes:

- `RunManager` owns bounded or detached one-off work whose durable result is a
  `Run`.
- `RuntimeManager` owns long-lived interactive interpreter processes whose value
  is their in-memory state between calls.

The existing `wisp-python` crate becomes `wisp-runtime`. Python and R are the two
explicitly supported languages. This is not a generic language-plugin system.

A runtime was originally project-scoped and execution-context-scoped, not
conversation-scoped, with the v1 identity:

```text
(project_id, context_id, language)
```

Issue #911 (parallel sessions bleeding into each other) forced a revision:
agent-driven runtimes are now additionally keyed by the owning conversation.
See the amendment in §6.1. There is at most one default runtime for each
identity. Code execution is serialized. Switching a conversation does not
destroy its runtime; deleting a conversation stops the runtimes it owns.

Runtime v1 persists only for the lifetime of the desktop/CLI process and its
transport connection. It does not survive an application restart or an SSH
disconnect. Cross-restart reattachment is a separate, substantially larger
feature.

## 2. Problem

Upstream scientific processing is naturally modeled as one-off Runs: submit work,
wait or poll, harvest outputs, and terminate the process. Downstream exploratory
analysis has different economics. Loading a multi-gigabyte data frame, matrix, or
model on every agent turn is slow and can dominate the analysis itself. A Python or
R process should load that data once and retain it in memory while the user explores
it over many calls.

The process must execute beside the data:

- Local data uses a local runtime.
- WSL data uses a runtime inside that WSL distribution.
- SSH-resident data uses a runtime on that SSH host.

Only source code, bounded text output, status, and explicit artifact references
should cross the control-plane connection. Large data must not be synchronized to
the desktop by default.

## 3. Current state

The current implementation provides useful pieces but has the wrong ownership for
this use case:

- `crates/wisp-python` provisions `app_data/python/.venv` with `uv`, launches
  `python/kernel_worker.py`, implements the JSON-lines client, and exposes the
  `python` tool.
- A `SessionRuntime` owns one Agent and therefore one Python kernel per conversation
  frame. Rebuilding that Agent rebuilds the kernel; separate conversations can load
  duplicate copies of the same large data.
- The Python kernel is launched only on the local host. It is not selected through
  `ExecutionContext`.
- `ExecutionContext` already models `local`, `wsl:<distro>`, and `ssh:<alias>` and
  the Run Manager already contains command-routing behavior for those context kinds.
- SSH connection parsing/credentials and the concrete `SshConnection` type currently
  live in the Tauri host, so the reusable runtime crate cannot depend on them
  directly.
- The app-managed Python environment is intentionally shared today: the Python REPL
  and bundled bio-tools MCP server are both launched with the interpreter from
  `app_data/python/.venv`. `python/requirements-mcp.txt` contains their combined
  dependencies. The CLI and MCP bridge use the same arrangement.
- Session export reads that same environment to capture installed Python packages.
- UI notebook projection, approval labels, provenance, bootstrap status, bundled
  resource paths, README text, and system prompts contain Python-specific cases.

The shared Python environment does not block the crate rename. It is a compatibility
constraint: bio-tools MCP may continue borrowing the managed Python interpreter,
but MCP process ownership remains in `wisp-mcp`/the host wiring and never moves into
`RuntimeManager`.

## 4. Goals

- Provide persistent Python and R sessions for exploratory analysis.
- Reuse one runtime across Agent reconstruction and conversation navigation within
  the same project.
- Run the interpreter in a selected Local, WSL, or SSH `ExecutionContext`.
- Keep data in its resident context and transmit only control messages and bounded
  outputs.
- Preserve the existing `python` tool contract for local calls.
- Add an equivalent `r` tool.
- Make process ownership explicit so runtimes can be listed, stopped, and restarted
  without leaking child processes.
- Keep Windows and macOS behavior explicit and testable without requiring real R,
  WSL, SSH, or network access.
- Deliver the change as small, independently testable PRs.

## 5. Non-goals for v1

- Surviving application restart, sleep, SSH disconnect, or host reboot.
- A detached remote runtime daemon, open TCP port, SSH tunnel, or reconnect protocol.
- Jupyter Server, Jupyter Kernel Gateway, or full Jupyter messaging compatibility.
- Interactive SLURM/PBS/LSF allocations. SSH v1 runs on the registered host; a
  scheduler-backed interactive context is a future `ExecutionContext` capability.
- Multiple named runtimes for the same project/context/language.
- Runtime pooling, automatic idle eviction, or automatic memory-pressure eviction.
- A variable browser, debugger, notebook replacement, or workspace inspector.
- Automatic serialization/checkpointing of Python or R memory.
- Automatic installation of R itself or arbitrary research packages.
- Project-directory synchronization or automatic transfer of large data.
- Representing every executed cell as a `Run` or research-graph node.
- A generic registry for future languages. Add one only when a third real language
  requires it.

## 6. Product and domain model

```text
Project
└─ ExecutionContext (local | wsl:<distro> | ssh:<alias>)
   ├─ Run                         one-off, persisted lifecycle
   └─ RuntimeSession              interactive, in-memory lifecycle
      ├─ Python worker
      └─ R worker

DataAsset / remote artifact reference
└─ context_id must match the runtime that consumes it
```

`RuntimeSession` is an operational object, not a new durable research-graph noun.
The durable scientific nouns remain `Project`, `ExecutionContext`, `DataAsset`,
`Run`, `Artifact`, `Paper`, and `Decision`.

### 6.1 Runtime key

```rust
struct RuntimeKey {
    project_id: String,
    scope_key: String,   // mainline | exploration id
    session_id: String,  // owning conversation frame; empty = scope-shared
    context_id: String,
    language: RuntimeLanguage,
}

enum RuntimeLanguage {
    Python,
    R,
}
```

The key excludes an environment/profile ID in v1: each key has one
configured/default interpreter. Changing the interpreter or package environment
requires a runtime restart. Named or profile-specific runtimes can be added
after a demonstrated need.

**Amendment (2026-08, issue #911).** The original v1 key deliberately excluded
the conversation/frame ID so one loaded dataset could be reused across
conversations. Parallel sessions in one project proved that unsafe: concurrent
conversations shared and clobbered each other's interpreter globals, which
escalated to overwritten output files and a recursively deleted sibling
working folder. The key now includes `session_id`, the owning conversation
frame:

- Agent `python`/`r` calls (including delegated subagents' child frames) run in
  a per-conversation interpreter. Within one conversation, state still persists
  across cells, turns, and Agent rebuilds.
- An empty `session_id` denotes a scope-shared runtime, used by user-driven
  UI commands when no conversation is being viewed.
- A deleted or transferred conversation stops its runtimes; `stop_scope`,
  `stop_project`, and shutdown behave as before and cover session runtimes.
- Cross-conversation reuse of one in-memory object is no longer implicit.
  Sharing large data between conversations should go through files or
  DataAssets, not interpreter globals.

### 6.2 Runtime status

The externally visible lifecycle is intentionally small:

```text
missing -> starting -> ready <-> busy -> stopping -> dead
```

- `starting`: transport and worker are being launched; the protocol handshake has
  not completed.
- `ready`: the worker accepts another cell.
- `busy`: one cell is executing; later calls wait in order.
- `stopping`: an explicit stop or application shutdown is terminating the process.
- `dead`: the worker exited, the transport failed, or the protocol became invalid.

`RuntimeInfo` should expose at least runtime ID, generation, key, status,
interpreter/version when known, start time, last activity time, best-effort resident
memory, and last error. Missing metrics are represented as unavailable, not zero.

The runtime ID identifies one process generation. `restart` creates a new ID or
increments a generation so logs never imply that old in-memory objects still exist.

## 7. Ownership and lifecycle

The desktop `AppState` owns one `RuntimeManager`. The CLI creates one manager for
its process lifetime. Agents and tools hold a shared manager handle; they do not own
worker processes.

`RuntimeManager` is responsible for:

- Looking up or lazily starting the runtime for a `RuntimeKey`.
- Ensuring concurrent starts for the same key create only one process.
- Serializing execution for each runtime.
- Owning the child process/SSH/WSL transport and the protocol I/O task.
- Tracking state, last error, and resource metrics.
- Listing, stopping, and restarting runtimes.
- Cleaning up all attached workers on normal application shutdown.

Lifecycle rules:

- Rebuilding an Agent does not restart a runtime.
- Switching, branching, or deleting a conversation does not stop a runtime.
- Switching projects does not automatically stop the old project's runtime; the
  Runtimes UI keeps it visible so a 10 GB session is not destroyed unexpectedly.
- Deleting a project stops its runtimes before project data is removed.
- Application shutdown attempts a clean stop, then kills remaining attached child
  processes.
- There is no idle timeout in v1. The user can see memory use and stop a runtime
  explicitly.
- Stop/restart is destructive to in-memory state and must be represented as such in
  the UI.
- A stop owns the worker's whole process tree, not just the direct child. An
  interpreter is often a launcher — Windows `Rscript.exe` re-launches
  `bin\x64\Rscript.exe` through `cmd.exe` — so the real interpreter and anything a
  cell started in the background go away with it.

Unlike the current `KernelClient`, the manager must retain the child handle rather
than intentionally forgetting it. Process cleanup and status detection require an
owned handle.

### 7.1 Execution queue and turn cancellation

The protocol reader/writer runs in a manager-owned background task. A tool call
submits work through a channel and awaits its result. Dropping or cancelling the
tool-call future must not drop the protocol read loop or desynchronize the next
cell.

There is one in-flight cell per runtime. If an Agent turn is cancelled while a cell
is running:

- v1 does not claim that the interpreter was interrupted;
- the manager continues draining the response and returns the runtime to `ready`;
- the abandoned result may be discarded, but any state mutation performed by the
  code remains in the runtime;
- the user can explicitly stop the runtime to terminate computation immediately,
  accepting loss of all in-memory state.

Reliable, state-preserving per-cell interrupt across Python, R, Windows, WSL, and
SSH is deferred.

## 8. `wisp-runtime` crate boundary

`crates/wisp-python` is renamed to `crates/wisp-runtime`; keeping both crates would
create an unnecessary compatibility layer.

`wisp-runtime` owns:

- `RuntimeManager`, runtime keys/status/info, and process generations.
- The language-neutral worker protocol and I/O driver.
- The launch specification/process contract consumed by `RuntimeManager`.
- Python and R worker resources and language-specific execution semantics.
- Discovery/selection of the interpreter used by a runtime.
- The existing app-managed Python environment bootstrap used as the default local
  Python interpreter.
- Agent tool adapters for `python` and `r`.

It does not own:

- Run submission, polling, cancellation, or artifact harvest.
- MCP transport, MCP process lifecycle, or connector registration.
- The persisted ExecutionContext registry, SSH credential/config parsing, or the
  concrete Tauri `SshConnection` type.
- Data transfer/synchronization.
- SQLite project/research records.
- Arbitrary project dependency installation.

The crate defines a narrow, mockable launcher boundary. The host resolves a context,
deploys the worker when necessary, and returns an attached process with stdin,
stdout, wait, and kill capabilities. `RuntimeManager` owns that returned process and
all later protocol/lifecycle behavior. This boundary is required both to avoid a
crate dependency on Tauri-only SSH state and to test without real processes.

No detailed source-file split is mandated by this spec. Modules should be separated
only where process management, host launching, protocol, or language-specific
behavior provides a real test/dependency boundary.

### 8.1 Shared app Python environment

For backward compatibility, the existing managed Python environment remains at
`app_data/python/.venv` in v1. Its requirements and marker behavior do not need to
change as part of the crate rename.

The crate rename also does not require moving the bundled `python/` resource
directory or changing the `WISP_KERNEL_WORKER` development override. R can add its
own bundled worker resource without forcing a data/resource migration unrelated to
runtime ownership.

Two independent consumers may request its executable path:

1. `RuntimeManager` uses it as the default interpreter for local Python sessions.
2. Desktop/CLI/MCP bridge wiring passes it to `wisp-mcp` for bundled bio-tools.

The second use does not make an MCP server a RuntimeSession. Starting, filtering,
restarting, or stopping interactive runtimes must not start or stop MCP servers.
Splitting service-Python and analysis-Python environments is deferred until package
conflicts, size, security, or update cadence creates a concrete need.

## 9. ExecutionContext transports

The desktop host implements the runtime launcher by resolving the existing
`ExecutionContext` and matching on `ExecutionContextKind`. It reuses the current SSH
registry/`SshConnection` and WSL conventions rather than duplicating credential or
context parsing inside `wisp-runtime`. The CLI supplies the equivalent launcher for
the contexts it supports.

All launchers present the same conceptual result to `RuntimeManager`: a worker with
piped stdin/stdout, a killable process handle, and termination reporting. This is a
fixed host boundary with fake implementations for tests, not a dynamic transport
plugin registry.

| Context | Launch behavior | Worker location | Lifetime |
|---|---|---|---|
| Local | Spawn interpreter directly with an argument vector | Bundled app resource | App process |
| WSL | `wsl.exe -d <distro> -- <interpreter> <worker>` | Versioned path inside the distro | WSL transport/app process |
| SSH | Existing `SshConnection` options + remote interpreter/worker command | Versioned path under `~/.wisp-science/runtime/` | SSH connection/app process |

### 9.1 Local Windows and macOS

- Spawn executable paths directly; do not construct a shell command containing
  user paths.
- Preserve existing Windows console-hiding behavior.
- Windows interpreter paths may contain spaces and must remain individual process
  arguments.
- A macOS GUI app may not inherit the user's interactive shell `PATH`. Runtime
  discovery must support an explicitly configured interpreter path and report an
  actionable error when discovery fails.
- Native Windows and WSL are separate contexts. Do not heuristically translate
  arbitrary paths. The one explicit bridge is the registered project root: WSL
  terminals, Runs, and interactive runtimes translate it with `wslpath`.

### 9.2 WSL and SSH worker deployment

The language workers are small versioned resources. Before launch, the host checks a
worker version/checksum at a context-local destination and uploads/writes the worker
only when absent or stale. Tests use fake launch/deployment runners; CI never
requires WSL or SSH.

The Tauri launcher reuses the registered alias/user/port/identity configuration
through the existing `SshConnection`. Private-key contents are never copied or
persisted by runtime deployment.

The v1 worker communicates only over inherited stdio. It opens no listening socket
or network port. When the transport receives EOF, the worker exits. An SSH failure
therefore marks the runtime `dead` and loses its in-memory state.

### 9.3 Working directory

- Local runtimes start in the project root.
- WSL runtimes start in the same project root translated with `wslpath`, because
  WSL shares the Windows filesystem and project-relative writes participate in
  local provenance, undo, and publication records.
- SSH runtimes use the context's configured default workdir when present,
  otherwise the probed context home/current directory.
- v1 does not create or synchronize an SSH mirror of the local project. Remote
  data should be addressed with paths meaningful inside that context.

A future project-to-context workspace binding may provide a stable remote project
root without changing runtime identity.

## 10. Interpreter and environment resolution

Interpreter selection is context-local. The host launcher resolves configured/probed
context information while the language adapter supplies language-specific defaults
and validation. The host must never use a local interpreter to execute against a
remote path.

Resolution order:

1. An explicitly configured, non-secret interpreter command/path in the
   `ExecutionContext` configuration.
2. A previously probed executable/capability for that context.
3. For local Python only, the existing app-managed uv interpreter.
4. For local R, PATH discovery of `Rscript` plus an actionable configuration error
   if it is unavailable.

Context probing should grow from a Python version hint to exact executable/version
hints for both Python and `Rscript`. WSL/SSH setup does not automatically install an
interpreter or research packages, because remote contexts may be offline, use
environment modules, or require administrator/scheduler policy.

The tool schema never accepts an arbitrary executable supplied by the model.
Interpreter selection is user/context configuration, not executable code input.

Project-specific pixi, conda, virtualenv, or `renv` profiles are future environment
selection work. A v1 user may explicitly configure the interpreter from such an
environment.

An explicitly configured interpreter must actually be launchable. When it lives
inside a conda-style prefix — a directory containing `conda-meta`, which conda,
mamba, and pixi all write — the worker is launched with that prefix on its own
`PATH`. On Windows an interpreter's shared libraries live in the prefix rather than
beside the executable, so without this a conda-forge `Rscript.exe` exits with
`STATUS_DLL_NOT_FOUND`, or faults on a mismatched library found elsewhere on `PATH`.
This is the launched child's environment only; the host environment is never
modified, and remote contexts keep whatever their own shell provides.

On Windows, `<R>\bin\Rscript.exe` is an architecture shim that re-launches
`<R>\bin\x64\Rscript.exe` through `cmd.exe`. Local launches resolve to the real
binary so the interpreter is the app's direct child.

## 11. Worker protocol

Python and R use one versioned, newline-delimited JSON protocol over stdio. Reusing
one protocol is smaller and safer than maintaining separate Rust clients.

Worker stdout is reserved exclusively for protocol frames. User stdout, stderr,
warnings, messages, and errors are captured and encoded inside frames. Worker
startup noise must not corrupt stdout.

### 11.1 Handshake

The worker sends a ready frame after initialization:

```json
{"type":"ready","protocol":1,"language":"python","pid":1234,"version":"3.x"}
```

The manager waits for this frame with a bounded startup timeout. Wrong protocol,
wrong language, malformed JSON, early EOF, or timeout fails startup and records an
actionable last error.

### 11.2 Execution

Request:

```json
{"type":"execute","id":"uuid","code":"..."}
```

Optional streaming frame:

```json
{"type":"stdout_chunk","id":"uuid","data":"..."}
```

Final response:

```json
{
  "type":"result",
  "id":"uuid",
  "stdout":"...",
  "stderr":"...",
  "error":null,
  "usage":{"wall_s":0.2,"cpu_s":0.1,"rss_kb":123456}
}
```

Every request/stream/result frame includes the request ID. Unknown frames may be
ignored only when the protocol version explicitly permits it; malformed frames on
the active request mark the runtime dead rather than risking state/result mismatch.

Buffered and streamed output retain explicit caps. Binary data, plots, and large
tables are written to context-local files and surfaced as artifact/data references,
not embedded in JSON.

### 11.3 Read-only object inspection

The UI may inspect an already-running runtime without executing user code or
starting a missing process:

```json
{"type":"inspect","id":"uuid"}
```

The worker returns at most 200 sorted, lightweight metadata rows and reports the
full count separately:

```json
{"type":"objects","id":"uuid","objects":[{"name":"counts","typeName":"DataFrame","summary":"12000000 × 48","sizeBytes":4294967296}],"totalCount":1}
```

Inspection shares the runtime execution queue, never reads full object contents,
and does not persist snapshots. Sizes are estimates; user-defined representations
and whole-object hashes are intentionally excluded.

### 11.4 Python semantics

The Python adapter preserves the current behavior:

- one persistent namespace;
- statements use `exec`, single expressions use `eval`, and non-`None` expression
  results are printed;
- stdout/stderr/error capture and bounded output;
- a non-interactive plotting backend so GUI windows cannot block the worker;
- optional resource metrics must not be required for successful execution.

The existing worker should be adapted to the versioned handshake/protocol rather
than replaced wholesale.

### 11.5 R semantics

The R adapter provides:

- one persistent evaluation environment;
- parsing/evaluation of multi-expression cells;
- printing of the final visible value using normal R visibility rules;
- separate capture of ordinary output, messages/warnings, and errors;
- traceback text where available;
- a non-interactive graphics policy; v1 requires explicit `png()`, `pdf()`,
  `ggsave()`, or equivalent artifact writes rather than opening GUI devices.

To reuse the JSON protocol without implementing a custom JSON parser in base R, the
v1 R worker requires `jsonlite`. Probe/startup checks it and returns a clear setup
error when absent. Runtime startup does not silently install it or access the
network.

## 12. Agent tools and host API

The existing `python` tool remains backward-compatible and can also execute a
saved project script without leaving the runtime:

```text
python(code? | script_path?, required_objects?, expected_runtime_generation?, context_id?)
```

- Exactly one of `code` or `script_path` is required. The schema keeps both
  optional and the host rejects a call that supplies neither or both, because
  tool schemas reach user-configured OpenAI-compatible gateways verbatim and a
  `oneOf` branch is not portable across them.
- `script_path` is a project-relative UTF-8 `.py` file; the host reads and hashes
  its exact content, then sends the content to the selected runtime. The file
  need not be copied to an SSH host — but the path is always resolved in the
  *local* project root, so a script that exists only on the remote host cannot be
  named this way. Read it into `code`, or fetch it into the project first.
- `context_id` is optional and defaults to `local`, preserving existing model calls.
- A non-local value must resolve to a registered ExecutionContext.
- The tool lazily ensures and reuses the runtime for its key.
- `required_objects` declares top-level binding names that must already exist;
  attribute or slot paths such as `adata.X` or `obj$slot` are not resolved. When
  non-empty, the tool never lazily starts an empty runtime, and it rejects a
  runtime started in another directory instead of falling back to a new one; the
  worker checks the bindings before evaluating any source.
- `expected_runtime_generation` optionally rejects a restarted/replaced runtime.

The new R tool mirrors it:

```text
r(code? | script_path?, required_objects?, expected_runtime_generation?, context_id?)
```

Its `script_path` must name a project-relative `.R` file. Script paths are source
artifacts, not process-launch instructions: both languages execute the saved
source inside their existing persistent namespace. Results from script execution
include the project-relative path, SHA-256, runtime id, and generation.

Tool descriptions state that variables and loaded data persist per
project/context/language, that package installation belongs to an explicitly chosen
project environment, and that paths are interpreted inside the selected context.

Approval previews must display both language and context so remote execution is not
mistaken for local execution.

Minimal host/UI commands:

- `list_runtimes(project_id?)`
- `start_runtime(project_id, context_id, language)`
- `stop_runtime(runtime_id)`
- `restart_runtime(runtime_id)`
- `execute_runtime(context_id, language, code)`

Agent execution uses the manager directly. `execute_runtime` was deferred until the
UI gained a code editor; the center file preview is now one, so it exists for the
script binding described in §13.1. It returns a small structured summary — the
console text plus any plot snapshots the cell produced as base64 PNGs
(`RuntimeExecutionSummary`). Code that raised is still a successful call, tagged
`[error]` by the same formatter the agent tools use. Because the user submits code
they are looking at, it does not route through agent tool approval.

## 13. UI

The Contexts surface gains a compact Runtimes section. Each row shows:

- project and execution-context label;
- Python or R;
- status;
- interpreter/version when known;
- best-effort resident memory;
- last activity;
- Start, Stop, or Restart actions appropriate to the state.

The UI must make destructive restart/stop semantics clear when a runtime has live
state. Script previews also expose a compact variable browser and execution
console; a package manager remains out of scope.

### 13.1 Script runtime binding

An `.R`/`.py` file open in the center preview binds to a runtime.
The language is fixed by the extension, so the binding is only a context choice:
the preview's picker lists contexts that can host that language, using the same
availability rule as the composer's `@` runtime entries. A stored binding that
cannot host the language is not honoured — it resolves to one that can, so the
picker's displayed context and the context a run is sent to never disagree. When
no context can host the language there is no binding or runtime inspector.

The source pane is directly editable for `.R`/`.py` workspace files: a
highlighted mirror sits under a transparent textarea, unsaved drafts are held
outside the component so an agent `FileChanged` remount cannot drop them, and
Ctrl+S (or the Save chip) persists through the workspace-scoped `save_file`
command — user-driven like `execute_runtime`, outside agent tool approval.
Files whose preview was truncated stay read-only, because saving a clipped
head would destroy the tail. The editor paints a live native textarea selection while focused, so dragging
across lines is not delayed until mouseup, and keeps a dedicated mark layer
for the same range after blur. It reports the selected line range in its
toolbar. Ctrl/Command+Enter runs the selection; with
a collapsed caret it runs the current statement — a parenthesized call, pipe
chain, or `if`/`function`/`def` body may span lines — and advances to the next
statement. A visible Run action exposes the same behavior without requiring the
shortcut. The editor toolbar has a whole-script Run action
(Ctrl/Command+Shift+Enter) beside the selection/statement Run. It saves a dirty
draft first, then calls
`execute_runtime_script` so the host reads and hashes the file on disk and
executes that content inside the bound persistent runtime — the same path the
agent `script_path` argument uses. A truncated preview stays read-only and has
no whole-script action, because saving a clipped head would destroy the tail.
Selecting code (in the editor's textarea or a read-only preview) continues to
expose the floating runtime execution action. Either UI execution path lazily
starts the runtime, so no separate Start click is needed. Selecting source and
choosing the AI handoff opens the existing conversation beside the preview,
where the agent can also make code changes.

The AI handoff retains both the selected text and its workspace path. The user
turn tells the agent that a change request targets that file, and the system
prompt requires a read-before-edit workflow: read the current file, apply a
focused exact replacement with `edit`, then verify the saved result instead of
only replying with a code block. Immutable artifacts and remote previews remain
reference-only. Successful `edit` and `write` calls emit a post-write
`FileChanged` event so an open center preview re-reads the file immediately;
the earlier diff-preview event is not used for refresh because it occurs before
the filesystem mutation.

The toolbar's runtime-inspector action opens an RStudio-style quadrant layout:
source top-left, an interactive console bottom-left, variables top-right, and a
plots pane bottom-right. The two dividers between the quadrants are draggable
(with an Escape cancel while dragging), resizing the right column and the
bottom row. Running selected code opens this layout and refreshes the
surfaces automatically. The console owns a visible multi-line prompt: Enter
runs the complete input, Shift+Enter adds a line, the adjacent Run action is an
equivalent mouse path, and ArrowUp/ArrowDown recall submitted single-line
history. Typed code runs against the same bound runtime as a selection run. The
plots pane shows the snapshots each execution
reported (base64 PNGs from the worker — matplotlib figure harvest on Python,
implicit-device `recordPlot` snapshot on R) with previous/next paging and a
clear action. Output does not enter the transcript. A user-driven run is not an
agent tool call, and persisting one as a `Role::Tool` message would emit a
`tool_result` with no matching `tool_use`, which the Anthropic API rejects.
Flooding the transcript with every REPL iteration would also re-send that
output on every later turn. The existing "add to chat" action remains the way
to hand a result to the agent. The console and plot history are in-memory for
the same reason the runtime is: a log that outlived its process would describe
variables that no longer exist. The console is a bounded bottom dock with its
own normal scroll surface and does not overlay the source.

R is optional. Missing `Rscript`/`jsonlite` is a capability state, not a global app
bootstrap failure. Existing Python bootstrap remains because the app-managed Python
environment is also used by bundled bio-tools MCP.

Notebook projection, syntax highlighting, approval labels, transcript input
extraction, provenance language mapping, system-prompt guidance, and i18n must
recognize the `r` tool alongside `python`.

## 14. Data locality and provenance

Runtime never moves input data implicitly. The controlling rule is:

```text
data residency context == runtime execution context
```

When typed DataAsset residency/context metadata is available, the host should reject
or explicitly stage a mismatch rather than guessing. Until that model is fully
typed, tool calls use explicit context-native paths and `context_id`.

Each runtime call remains a tool execution in the originating conversation, not a
`Run`. Its code or script path, context input, and bounded output continue to be
persisted with that tool call. Script results additionally record the content hash,
runtime ID, and generation, so later file edits do not erase which source executed.
Runtime v1 does not add a durable cross-conversation execution journal solely to
duplicate persisted tool messages. Consequently, v1 does not claim complete replay
of shared in-memory state assembled across multiple conversations.

Files created remotely remain remote references unless the user or an output spec
explicitly downloads them. Runtime does not recursively snapshot a remote filesystem
to infer every read/write.

Runtime does not automatically serialize Python globals, R workspaces, or a 10 GB
object graph. Rehydration scripts and explicit durable intermediate formats are user
artifacts, not hidden runtime checkpoints.

## 15. Failure handling and security

- Start failure leaves no apparently-ready runtime and records the underlying
  interpreter, deployment, transport, dependency, or handshake error.
- Unexpected worker/transport exit marks the runtime dead and fails queued calls.
- Protocol corruption marks the runtime dead; it is never repaired by skipping the
  active response blindly.
- A later execute against a dead runtime may require explicit Restart rather than
  silently claiming old state survived. Lazy start applies to missing runtimes, not
  dead generations.
- SSH disconnect is terminal in v1.
- Stopping a runtime closes stdin so the worker can exit on its own, then kills it,
  then terminates its whole process tree. Every step has a deadline, and the total
  budget for one stop request is shared across the runtimes it covers: a worker that
  refuses to exit must never block an Agent turn, a project switch, or app exit.
  Stopping therefore also reclaims a background process a cell left running.
- Arbitrary Python/R execution continues to use the existing approval system.
- Code travels over inherited local/WSL/SSH stdio, not an unauthenticated listening
  port.
- SSH credentials remain in the existing SSH/keyring paths; private key contents are
  never stored in SQLite or copied by runtime deployment.
- Runtime output caps prevent accidental unbounded transcript/memory growth.

## 16. Testing

Automated tests must not require R, a real SSH host, WSL, a GPU, a scheduler, an API
key, or network access.

### 16.1 Runtime manager tests

- Concurrent `ensure` calls for one key launch exactly one fake worker.
- Two executes for one runtime are ordered and never overlap.
- Different keys can execute independently.
- Agent/conversation detach does not destroy the runtime.
- Stop/restart replaces the process generation and clears state.
- Worker exit/protocol error transitions to dead and fails queued calls.
- Dropping a caller does not drop the manager-owned protocol loop.

### 16.2 Protocol tests

- Ready handshake success, timeout, wrong version/language, malformed frame, and EOF.
- Stream chunks and final result correlate by request ID.
- Output caps are enforced.
- Fake Python and R workers demonstrate persistent state without invoking real
  interpreters.

### 16.3 Transport tests

- Pure command builders cover Local, WSL, and SSH argument vectors.
- Windows paths with spaces remain single arguments.
- WSL distro and SSH connection configuration are preserved.
- Fake deployment runner verifies checksum hit/miss behavior.
- SSH drop and worker exit are reported as terminal.

### 16.4 Integration/UI tests

- Existing local `python(code)` behavior remains compatible.
- `python`/`r` route to the requested context through a fake manager.
- UI Playwright mocks cover ready/busy/dead rows and stop/restart actions.
- Notebook/provenance projections classify R cells as R.

Relevant narrow checks run first, followed by the repository-required formatting,
workspace tests, WASM check, and Playwright suite for implementation PRs that touch
UI/Tauri behavior.

## 17. Incremental delivery

Each item is a separate reviewable change; do not implement later items early.

### PR 1: Rename without behavior change

- Rename `wisp-python` to `wisp-runtime` and update workspace dependencies, docs,
  desktop, CLI, MCP bridge, and session export references.
- Preserve the current local Python REPL and shared uv environment behavior.
- Preserve `app_data/python`, the bundled `python/` resource path, and
  `WISP_KERNEL_WORKER`; a crate rename is not a resource/data migration.
- Do not add R or remote launching.

### PR 2: RuntimeManager and local Python ownership

- Add the runtime key/status/manager and manager-owned protocol task.
- Move local Python kernel ownership out of conversation `SessionRuntime`.
- Preserve the existing `python` tool schema and local default.
- Add manager/protocol lifecycle tests.

### PR 3: ExecutionContext transports

- Add interpreter capability probes and a host-provided launcher boundary.
- Implement the Tauri Local/WSL/SSH launcher using the existing context and SSH
  registry types.
- Add versioned worker deployment for WSL/SSH.
- Extend `python` with optional `context_id`.
- Use fake runners; do not require a real remote host.

### PR 4: R adapter

- Add R capability probing, worker, `r` tool, output/error semantics, and tests.
- Keep R optional and do not auto-install R or packages.

### PR 5: Runtime UI and remaining integration

- Add runtime list/status/stop/restart UI.
- Extend notebook, provenance, approval, prompts, i18n, and mocked Playwright tests.

## 18. Acceptance scenarios

### Local exploratory Python

On Windows or macOS, a user loads a large local data object in `python`. Later calls
from the same project reuse it without rereading the file. Switching conversations
and returning does not duplicate or destroy the runtime. The Runtimes UI shows its
memory and can stop it explicitly.

### Remote exploratory Python

A project has `ssh:omics` registered. The user invokes Python with that context and
loads a 10 GB remote dataset by its remote path. The Python process and data remain
on `ssh:omics`; only code and bounded output cross SSH. Subsequent calls reuse the
same remote in-memory object while the connection remains alive.

### Local or remote R

The selected context exposes `Rscript` and `jsonlite`. The user loads data into a
persistent R environment, evaluates later cells against it, captures warnings and
errors, and explicitly writes plots as context-local artifacts.

### Explicit v1 lifetime boundary

After application restart or SSH disconnect, the previous runtime is not presented
as reusable. Its status is absent/dead and starting again creates a new generation;
the product never implies that lost memory survived.

## 19. Deferred upgrades and their triggers

- **Streamed console output:** `execute_runtime` buffers and returns the final frame.
  Stream `RuntimeEvent::Stdout` to the console when cells run long enough that no
  feedback until completion is a measured problem.
- **Concurrent script runs:** the preview allows one run at a time across all files.
  Key its busy state by file when two runtimes on different contexts need to run at
  once.
- **Named/multiple runtimes:** add when users need two independent environments for
  one project/context/language and the single-default rule becomes limiting.
- **Detached remote supervisor/reconnect:** add when losing state across routine SSH
  interruption or desktop restart is a measured problem. Evaluate established
  Jupyter kernel/server protocols before designing a proprietary daemon.
- **Scheduler interactive runtimes:** add when users need exploratory memory on
  compute nodes rather than SSH login hosts; model the allocation through
  ExecutionContext/Run lifecycle rather than hiding `salloc` inside the worker.
- **Environment profiles:** add when one configured interpreter per key is
  insufficient; integrate explicit pixi/conda/venv/renv identities.
- **State checkpoint/replay:** add only for explicit workflows with acceptable
  serialization cost and side-effect semantics; never silently snapshot arbitrary
  10 GB sessions.
- **Third language/plugin registry:** add when a real third adapter exists.
