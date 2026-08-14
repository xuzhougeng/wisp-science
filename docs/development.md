# Development

Build, architecture, CLI environment, and tests. For first-run desktop setup see
[basic configuration](basic-configuration.md). For HTTP model profiles see
[model configuration](model-configuration.md).

## Build from source

Prerequisites:

- **Rust** (stable, 1.88+) with `wasm32-unknown-unknown`:
  `rustup target add wasm32-unknown-unknown`
- **uv**: <https://docs.astral.sh/uv/>
- **Trunk**: `cargo install --locked trunk`
- **Tauri CLI v2**: `cargo install tauri-cli --version "^2"`
- Optional: **R** with `jsonlite` for the persistent `r` tool. Wisp locates
  `Rscript` via Settings, then PATH, then well-known install locations
  (for example `C:\Program Files\R\R-*\bin` on Windows). It never installs R
  packages automatically.
- Windows needs the **WebView2 Runtime** (present on most Windows 10/11
  systems; the installer acquires it when missing). macOS needs **Xcode
  Command Line Tools** (`xcode-select --install`) and uses system WebKit.

```bash
cargo tauri dev      # hot-reload: Trunk serves the UI, Tauri opens the window
cargo tauri build    # installers under target/release/bundle
```

Universal macOS binary (Apple Silicon + Intel):

```bash
rustup target add x86_64-apple-darwin
cargo tauri build --target universal-apple-darwin
```

### Windows launch troubleshooting

If the window never appears after install, **Quit** from the tray icon and
repair the [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/#download-section)
(Evergreen Standalone Installer, run as administrator), then reopen Wisp.

Packaged Windows builds have no console. Each launch writes
`%APPDATA%\science.wisp-science\wisp-science\logs\wisp.log` (overwritten on the
next launch). The `startup finished` line breaks pre-first-paint work by phase
(`total=…ms store=…ms skills=…ms …`). Recovery sweeps, the scratch sandbox
purge, and restoring project windows run after the window is interactive and
are logged as `deferred startup finished`.

## Headless CLI

```bash
export WISP_API_KEY=<your provider key>
export WISP_PROVIDER=openai            # openai | openai_responses | anthropic
export WISP_MODEL=deepseek-v4-flash
cargo run -p wisp-cli                  # interactive agent
cargo run -p wisp-cli -- run "Summarize the files in this project"
cargo run -p wisp-cli -- run --output jsonl "Summarize the files in this project"
```

Eval and the long-lived JSONL RPC protocol:
[headless agent testing](headless-agent-testing.md).

### Environment variables

| Variable             | Purpose                                                       |
|----------------------|---------------------------------------------------------------|
| `WISP_API_KEY`       | Provider API key (CLI). Desktop uses the OS keyring.          |
| `WISP_PROVIDER`      | CLI API provider: `openai` (default), `openai_responses`, or `anthropic` |
| `WISP_API_URL`       | API root; defaults to DeepSeek / OpenAI / Anthropic           |
| `WISP_MODEL`         | Model name                                                    |
| `WISP_MAX_CONTEXT`   | Context budget (default 1,000,000)                            |
| `WISP_MAX_ITER`      | Max agent iterations per turn (default 100; 0 = unlimited)    |
| `WISP_SKILLS_PATH`   | Extra `;`/`:`-separated SKILL.md catalog dirs                 |
| `WISP_KERNEL_WORKER` | Override path to `kernel_worker.py` (bundled by default)      |
| `WISP_MCP_COMMAND`   | Launch an arbitrary stdio MCP server (full command line)      |
| `WISP_MCP_PKG`       | Launch a bundled bio-tools server, e.g. `mcp_pubmed`          |

Desktop stores API keys in the OS keyring and model profiles in
`.wisp/wisp.sqlite`. Custom credentials map a display name to an environment
variable and are injected only into newly launched local Python and bundled MCP
processes — never copied to SSH/WSL hosts.

Wisp reads `AGENTS.md` from the project root when a new session starts.
Instructions in **Project Settings → Agent Context** live in `.wisp/WISP.md`
and take precedence when both exist.

### Bundled bio-tools MCP

`WISP_MCP_PKG=mcp_pubmed` launches `mcp-servers/bio-tools/run_server.py
mcp_pubmed` inside the uv venv. Install the server's dependencies first:

```bash
uv pip install mcp requests
# plus any server-specific deps (httpx, xmltodict, etc.) the package imports
```

The agent discovers matching tools with `search_mcp_tools` and calls the
selected one through `use_mcp_tool`; the full server catalog is never copied
into every model request.

Desktop users add remote MCP (Notion, Parallel Search, …) under
**Settings → Connections**. See [basic configuration](basic-configuration.md).

## Repository layout

```
wisp-science/
├─ crates/
│  ├─ wisp-llm/     Provider trait + OpenAI-compatible + Anthropic + SSE + RoutedProvider
│  ├─ wisp-core/    ContextManager (3-tier compaction), SystemPrompt, agent_loop, memory
│  ├─ wisp-tools/   read/write/edit/search/grep/shell/attempt_completion + Windows safety
│  ├─ wisp-store/   sqlx SQLite (projects/frames/messages/artifacts/settings) + OS keyring
│  ├─ wisp-skills/  SKILL.md discovery + search_skills/use_skill progressive loading
│  ├─ wisp-runtime/ project-scoped Python/R runtime manager + REPL tools
│  ├─ wisp-mcp/     stdio JSON-RPC MCP client + McpTool adapter (bundled bio-tools)
│  ├─ wisp-acp/     ACP v1 stdio client for external coding agents
│  ├─ wisp-sync/    Encrypted snapshot protocol + self-hosted relay server
│  └─ wisp-cli/     `wisp-science` headless binary
├─ src-tauri/       Tauri v2 desktop shell (commands + agent event stream)
├─ ui/              Leptos CSR frontend (built by Trunk, loaded in WebView2)
├─ python/          kernel_worker.py + mock MCP server (uv-managed)
├─ r/               optional system-R kernel worker (requires jsonlite)
├─ skills/          Bundled SKILL.md catalog for reusable scientific workflows
├─ mcp-servers/     Bundled MCP servers (bio-tools: ~80 DB clients)
└─ seed/            Bundled demo session recordings (ESR1 / GSE153250 ×5)
```

## Architecture

- **Agent loop** (`wisp-core::agent`): read → think → tool-call → verify,
  streaming tokens to an `Output` sink. Stops on `attempt_completion` or when
  the model returns no tool calls.
- **Context compaction** (`wisp-core::context`): an archive-first pipeline fires
  before each model call at 80% of the context budget — prune tool/media noise
  older than the protected recent agent rounds, then summarize sanitized
  history, keeping one incremental checkpoint plus an 8K-token recent tail. The
  post-compact target adapts to measured per-iteration growth instead of a
  fixed percentage, and a failed attempt suppresses automatic retries until the
  estimate grows further. Old turns are never silently dropped.
- **Providers** (`wisp-llm`): one trait, two wire formats (OpenAI
  `/chat/completions` and Anthropic `/v1/messages`), both with SSE streaming.
  `RoutedProvider` picks a low/medium/high tier per turn.
- **Tools** (`wisp-tools`): filesystem + shell tools with Windows-aware
  dangerous-command gating and a path sandbox rooted at the project directory.
- **Python/R REPLs** (`wisp-runtime`): one manager-owned process per
  project/context/language keeps its namespace across cells and conversations;
  local, WSL, and SSH contexts share one versioned protocol.
- **MCP** (`wisp-mcp`): a newline-JSON-RPC client launches any stdio MCP
  server; remote schemas stay behind `search_mcp_tools` / `use_mcp_tool`
  until a task needs them.

## Testing

- **Rust unit tests** — `cargo test --workspace`
- **MCP client smoke** — `cargo run -p wisp-mcp --example smoke` launches the
  bundled mock MCP server via `uv` and round-trips `tools/list` + `tools/call`.
- **UI E2E (Playwright + Tauri mock)** — `ui-tests/` runs the Leptos UI in a
  headless browser against `trunk serve`, with a mocked `window.__TAURI__`:

  ```bash
  cd ui-tests
  npm install
  npx playwright install chromium
  npx playwright test
  ```

## Roadmap

- `FlashThinking` — phase-aware structured thinking-framework injection.
- `loop_engine` — deeper Implementer / Verifier / Updater workflows beyond the
  bounded automatic Reviewer pass shipped today.
- Richer artifact management, including an embedded Mol* 3D structure viewer.
- `RoutedProvider` LLM-score tier selection (keyword routing is already wired).

## Third-party attributions

- Real-browser automation is inspired by
  [GenericAgent's GA Web / TMWebDriver](https://github.com/lsdefine/GenericAgent)
  (MIT, Copyright 2025 lsdefine). Wisp's Rust bridge and Manifest V3 extension
  are an independent implementation; see
  [`browser-extension/NOTICE.md`](../browser-extension/NOTICE.md).
- The agent core is based on
  [`w4n9H/mangopi-cli`](https://github.com/w4n9H/mangopi-cli) (Apache-2.0).
- `skills/` and `mcp-servers/bio-tools/` vendored from the upstream
  `wisp-science` asset bundle (Apache-2.0).
- `skills/bear-*` from [bear-research-skills](https://github.com/fei0810/bear-research-skills)
  (CC BY-NC-SA 4.0); requires `scimaster-cli` for live retrieval.
- `kernels/kernel_worker.py` protocol adapted from the upstream operon kernel
  worker, with POSIX-only `resource`/`/proc`/`SIGINT` machinery dropped for
  Windows.
