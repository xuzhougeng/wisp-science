# wisp-science

[English](README.md) | [简体中文](README_zh.md)

> **Our manifesto:** Wisp Science is open source and borderless. We are building
> a scientific workbench that anyone, anywhere can use, study, improve, and
> share.

<p>
<a href="https://github.com/xuzhougeng/wisp-science/releases"><img src="https://img.shields.io/badge/Windows-supported-0078D4" alt="Windows supported"></a>
<a href="https://github.com/xuzhougeng/wisp-science/releases"><img src="https://img.shields.io/badge/macOS-supported-000000" alt="macOS supported"></a>
<a href="#build--run"><img src="https://img.shields.io/badge/Linux-source%20build-FCC624" alt="Linux source build"></a>
<a href="https://github.com/xuzhougeng/wisp-science/blob/main/LICENSE"><img src="https://img.shields.io/github/license/xuzhougeng/wisp-science" alt="License"></a>
<br>
<a href="https://github.com/xuzhougeng/wisp-science/stargazers"><img src="https://img.shields.io/github/stars/xuzhougeng/wisp-science?style=social" alt="Stars"></a>
</p>

wisp-science is a local-first desktop copilot for science: it talks to any
OpenAI-compatible or Anthropic model, runs persistent Python and R REPLs, calls
tools on the local filesystem, loads reusable `SKILL.md` workflows, and
reaches ~80 biological databases through bundled MCP servers — all from a
Tauri v2 desktop window (WebView2 on Windows, system WebKit on macOS) or a
headless CLI.

> Status: MVP vertical slice. The agent loop, streaming providers, tools,
> Python/R REPLs, SQLite store, MCP client, and Leptos UI all build and run.
> See [Roadmap](#roadmap) for what is deferred.

## Layout

```
wisp-science/
├─ crates/
│  ├─ wisp-llm/     Provider trait + OpenAI-compatible + Anthropic + SSE + RoutedProvider
│  ├─ wisp-core/    ContextManager (3-tier compaction), SystemPrompt, agent_loop, memory
│  ├─ wisp-tools/   read/write/edit/search/grep/shell/attempt_completion + Windows safety
│  ├─ wisp-store/   sqlx SQLite (projects/frames/messages/artifacts/settings) + OS keyring
│  ├─ wisp-skills/  SKILL.md discovery + use_skill tool (bundled catalog at skills/)
│  ├─ wisp-runtime/ project-scoped Python/R runtime manager + REPL tools
│  ├─ wisp-mcp/     stdio JSON-RPC MCP client + McpTool adapter (bundled bio-tools)
│  ├─ wisp-acp/     ACP v1 stdio client for external coding agents
│  ├─ wisp-sync/    Encrypted snapshot protocol + self-hosted relay server
│  └─ wisp-cli/     `wisp-science` headless binary
├─ src-tauri/       Tauri v2 desktop shell (commands + agent event stream)
├─ ui/              Leptos CSR frontend (built by Trunk, loaded in WebView2)
├─ python/          kernel_worker.py + mock MCP server (uv-managed)
├─ r/               optional system-R kernel worker (requires jsonlite)
├─ skills/          Bundled SKILL.md catalog (29 science workflows)
├─ mcp-servers/     Bundled MCP servers (bio-tools: ~80 DB clients)
└─ seed/            Bundled demo session recordings (CRISPR / enzyme / extremophile / immunotherapy)
```

## Prerequisites

- **Rust** (stable, 1.88+) with `wasm32-unknown-unknown`:
  `rustup target add wasm32-unknown-unknown`
- **uv** (Python environment manager): <https://docs.astral.sh/uv/>
- Optional: **R** with `Rscript` on PATH and the `jsonlite` package for the
  persistent `r` tool. Wisp never installs R packages automatically.
- **Trunk** (WASM frontend bundler): `cargo install --locked trunk`
- **Tauri CLI v2**: `cargo install tauri-cli --version "^2"`
- **WebView2 Runtime** (Windows only) — preinstalled on Windows 10/11; the
  installer bundles it on demand.
- **Xcode Command Line Tools** (macOS only): `xcode-select --install` — macOS
  uses the system WebKit, so no extra runtime is needed.

## Build & run

### Headless CLI

```powershell
$env:WISP_API_KEY = "<your provider key>"
$env:WISP_PROVIDER = "openai"           # openai=OpenAI-compatible Chat Completions; or openai_responses / anthropic
$env:WISP_MODEL     = "deepseek-v4-pro" # openai_responses: gpt-5.5; anthropic: claude-sonnet-5
cargo run -p wisp-cli
```

The CLI auto-loads the bundled `skills/` catalog and wires the bundled Python
and optional system-R REPLs. Python provisions a uv venv at
`.wisp/python/.venv` on first run; R uses `Rscript` from PATH and requires
`jsonlite` in that R environment. In the desktop app, Python and R interpreter
paths are saved per execution context from the Contexts panel or the agent's
`set_runtime_interpreter` tool, so `local`, WSL, and each SSH server can use
different environments without host environment variables. The tool restarts
the current project's matching REPL when needed, so a failed runtime can recover
without restarting the Wisp app; restarting clears that REPL's in-memory state.
The composer compute button opens the fixed host picker with Local first and the
configured SSH hosts below it. Selecting one host then opens an environment card
for only that context, with its probe summary, runtime/run counts, and shortcuts
to details, probing, and a terminal.
Each Python or R cell is limited to 1 MiB of source so a malformed request cannot
exhaust the persistent worker before execution begins.

### Desktop app

```powershell
cargo tauri dev      # hot-reload: Trunk serves UI, Tauri opens WebView2
cargo tauri build    # produce an MSI/NSIS installer under target/release/bundle
```

Desktop development uses port `1421`. UI tests use `1422`, and their Trunk
outputs are isolated in `ui/dist-dev` and `ui/dist-test`; release packaging
continues to use `ui/dist`. This prevents a running dev/test server from racing
with `cargo tauri build` while it copies the optimized WASM bundle.

On macOS, run the same commands from a shell (`cargo tauri build` emits a
`.app` and `.dmg` under `target/release/bundle`). `src-tauri/tauri.macos.conf.json`
is auto-merged by Tauri to replace the PowerShell `beforeBuildCommand` with a
cross-platform `trunk build`. For a universal binary (Apple Silicon + Intel):

```bash
rustup target add x86_64-apple-darwin
cargo tauri build --target universal-apple-darwin
```

The `.app`/`.dmg` are unsigned — first launch needs right-click → Open (or
allow it in System Settings → Privacy & Security).

The desktop app stores API keys in the OS keyring and model profiles in
`.wisp/wisp.sqlite` (Settings -> Models). Profiles can point at remote API
providers. See [Model configuration](docs/model-configuration.md) for the
provider fields. **Conversations persist to that SQLite database** — each turn's
messages are appended to the active session frame, so restarting the app
restores the full history. The headless CLI keeps using `.wisp/session.json` for
portability.

Projects can be moved between Windows and macOS from the Projects screen. Use
the download action on a project card to export a versioned ZIP, then **Import
project** on the other computer. The importer asks for a parent folder and
creates a new project directory there; Windows drive letters are never reused.
See [Project transfer](docs/project-transfer.md) for contents and limitations.

Projects can also be synchronized explicitly between devices. Configure either
a self-hosted relay or a folder managed by the Baidu Netdisk/Nutstore desktop
client in **Settings → General**, then press **Sync now** on a project card.
Synchronization never runs in the background and refuses to start while a task,
approval, review, or run is active. Project contents are encrypted before they
reach either backend; workspace files are uploaded incrementally by content.
See [Manual project sync](docs/project-sync.md) or the
[Chinese sync guide](docs/project-sync.zh-CN.md) for setup, device codes,
conflicts, path behavior, relay deployment, and limitations.

### Local ACP Agents

Wisp can launch any already-installed local agent that speaks stable ACP v1
over stdio. This is separate from **Settings → Models** (HTTP API profiles).

Quick path:

1. Install an ACP adapter, for example Codex:
   `npm install -g @agentclientprotocol/codex-acp`
2. Open **Settings → Models → ACP Agents**, or from the chat model picker click
   **Add ACP Agent**. Do not put ACP launch commands in the HTTP “Add model” form.
3. Set **Label**, **Command** (`codex-acp` or `npx` / `npx.cmd`), and
   **Arguments** (one per line; for `npx` use `-y` then
   `@agentclientprotocol/codex-acp`).
4. **Save Agent** → **Test Connection** → authenticate if offered.
5. Select the agent and send a prompt. If the current conversation already has
   messages, Wisp starts a new empty session automatically because ACP cannot
   rebind existing transcript history. The selection locks after the first
   message.

Do not use plain `codex` / `claude` here — they are not ACP. Use an adapter
such as [`codex-acp`](https://github.com/agentclientprotocol/codex-acp) or
[`claude-agent-acp`](https://github.com/agentclientprotocol/claude-agent-acp).

Full setup, Claude example, Windows notes, and troubleshooting:
[docs/acp-agents.md](docs/acp-agents.md).

### Composer references and search

In a desktop conversation, type `@` to attach a saved artifact, `#` to attach
a saved session (including another project), or `/` to apply an enabled skill
to the next turn. Attachments are explicit, removable chips; cross-project
artifacts stay at their original local path and are never copied automatically.
The same references work with ACP Agents: selected skills and session context
are sent as ACP text blocks, while artifacts are sent as file links.

Use Ctrl+K on Windows/Linux or Cmd+K on macOS to search projects, artifacts,
sessions, and common commands. Enter opens the selected result; Shift+Enter
attaches an artifact or session to the composer.

Saved conversations and conversation folders expose visible action buttons in
the sidebar on macOS, Windows, and Linux. Use them to rename or delete folders,
or to rename, organize, copy, move, export, or delete a conversation. The
sidebar loads the newest 100 conversations first; use **Load earlier sessions**
to fetch older pages. Opening a conversation initially loads its newest 20 user
turns; use **Load earlier messages** at the top of the transcript to fetch older
complete turns without splitting tool calls from their results. The chat mounts
at most 40 complete user turns at once; use the earlier/newer controls to move
through already loaded history without growing the DOM unboundedly. Remote
file rows also expose a visible download action, while secondary-click remains
available as an alternate path. Cross-project transfer copies the saved
transcript only. Project files and runs remain in their source project;
conversation-linked artifact records are not transferred, and the underlying
workspace files are never deleted.

On macOS, the native app menu mirrors the global desktop command surface,
including project navigation, new-session commands, edit shortcuts, and
`Check for Updates…`. Row-specific conversation and folder actions stay beside
their rows. The same update check is also available from the Settings page and
the Windows in-window Help menu. It
now reports the result in an in-app dialog, including whether you are already
up to date or a newer release is available on GitHub Releases.

### Bundled demos

## Configuration

All optional; sensible defaults are bundled.

| Variable             | Purpose                                                       |
|----------------------|---------------------------------------------------------------|
| `WISP_API_KEY`       | Provider API key (CLI). Desktop uses the keyring instead.     |
| `WISP_PROVIDER`      | CLI API provider: `openai` (default), `openai_responses`, or `anthropic` |
| `WISP_API_URL`       | API root; defaults to DeepSeek / OpenAI / Anthropic           |
| `WISP_MODEL`         | Model name                                                    |
| `WISP_MAX_CONTEXT`   | Context budget (default 1,000,000)                            |
| `WISP_MAX_ITER`      | Max agent iterations per turn (default 100)                   |
| `WISP_SKILLS_PATH`   | Extra `;`/`:`-separated SKILL.md catalog dirs                 |
| `WISP_KERNEL_WORKER` | Override path to `kernel_worker.py` (bundled by default)      |
| `WISP_MCP_COMMAND`   | Launch an arbitrary stdio MCP server (full command line)      |
| `WISP_MCP_PKG`       | Launch a bundled bio-tools server, e.g. `mcp_pubmed`          |

### Bundled bio-tools MCP

`WISP_MCP_PKG=mcp_pubmed` launches `mcp-servers/bio-tools/run_server.py
mcp_pubmed` inside the uv venv. The venv must have the server's dependencies
installed first:

```powershell
uv pip install mcp requests
# plus any server-specific deps (httpx, xmltodict, etc.) the package imports
```

Then the agent can call that server's tools (e.g. PubMed search) directly.

### Bundled demos

`seed/` ships four pre-baked example sessions (CRISPR screen, enzyme
engineering, extremophile, immunotherapy) recorded from the upstream agent.
In the desktop app, **Open demo** lists them and opens one as a read-only
User + Assistant transcript. Bundled `assets_*.tar.gz` archives are extracted
into the workspace on open so figures and data files in the right panel preview
correctly.

## Testing

- **Rust unit tests** — `cargo test --workspace`
  (covers `wisp-store` SQLite round-trips, the seed demo loader, etc.).
- **MCP client smoke** — `cargo run -p wisp-mcp --example smoke` launches the
  bundled mock MCP server via `uv` and round-trips `tools/list` + `tools/call`.
- **UI E2E (Playwright + Tauri mock)** — `ui-tests/` runs the Leptos UI
  in a headless browser against `trunk serve`, with a mocked
  `window.__TAURI__` so no Rust backend or API key is needed:

  ```powershell
  cd ui-tests
  npm install
  npx playwright install chromium      # one-time browser download
  npx playwright test                  # serve UI + run the full mocked desktop flow suite
  ```

  The mock (`tests/mock-tauri.ts`) stubs `invoke`/`listen` with canned data
  and even simulates a streamed assistant turn, so the tests exercise the real
  Leptos rendering and event handling without touching the network.

## Architecture

- **Agent loop** (`wisp-core::agent`): read → think → tool-call → verify,
  streaming tokens to an `Output` sink. Stops on `attempt_completion` or when
  the model returns no tool calls.
- **Context compaction** (`wisp-core::context`): three tiers fire before each
  model call at 80% of the context budget — micro-compact oversized tool
  output, drop old turns, then an LLM-driven full summary as a last resort.
- **Providers** (`wisp-llm`): one trait, two wire formats (OpenAI
  `/chat/completions` and Anthropic `/v1/messages`), both with SSE streaming.
  `RoutedProvider` picks a low/medium/high tier per turn from the last user
  message.
- **Tools** (`wisp-tools`): filesystem + shell tools with Windows-aware
  dangerous-command gating and a `dunce`-canonicalized path sandbox rooted at
  the project directory.
- **Python/R REPLs** (`wisp-runtime`): one manager-owned process per
  project/execution context/language keeps its namespace across cells and
  conversations; local, WSL, and SSH contexts use the same versioned protocol.
  R is optional and uses an existing `Rscript` plus `jsonlite`. The Contexts
  panel probes interpreter capabilities; selecting a local, WSL, or SSH server
  reveals only that context's runtimes and runs in the detail pane. Runtime
  details include status, memory, last activity, destructive Stop/Restart
  controls, and an on-demand read-only list of in-memory object names, types,
  shapes/sizes, and bounded metadata.
- **MCP** (`wisp-mcp`): a minimal newline-JSON-RPC client launches any stdio
  MCP server and exposes each remote tool as a first-class agent tool.

## Acknowledgements

- **Claude Science (Operon)** is referenced in product comparison and
  compatibility research.
- The agent core is based on
  [`w4n9H/mangopi-cli`](https://github.com/w4n9H/mangopi-cli) (Apache-2.0).
- `skills/` and `mcp-servers/bio-tools/` vendored from the upstream
  `wisp-science` asset bundle (Apache-2.0).
- `skills/bear-*` from [bear-research-skills](https://github.com/fei0810/bear-research-skills)
  (CC BY-NC-SA 4.0); requires `scimaster-cli` for live retrieval.
- `kernels/kernel_worker.py` protocol adapted from the upstream operon kernel
  worker, with POSIX-only `resource`/`/proc`/`SIGINT` machinery dropped for
  Windows.

See `LICENSE` (Apache-2.0). Upstream notices are preserved in their respective
directories.

## Citation

If you use wisp-science in your research, please cite:

[![DOI](https://zenodo.org/badge/1285857639.svg)](https://doi.org/10.5281/zenodo.21193742)

```bibtex
@software{xu2026wisp,
  author    = {Xu, Zhougeng and hoptop},
  title     = {wisp-science: A local-first scientific computing agent},
  version   = {v0.4.1},
  year      = {2026},
  publisher = {Zenodo},
  doi       = {10.5281/zenodo.21193742},
  url       = {https://doi.org/10.5281/zenodo.21193742}
}
```

## Roadmap (post-MVP)

- `FlashThinking` — phase-aware structured thinking-framework injection.
- `loop_engine` — deeper Implementer / Verifier / Updater workflows beyond the
  bounded automatic Reviewer pass shipped today.
- Artifact management + inline Mol* 3D structure viewer in the UI.
- `RoutedProvider` LLM-score tier selection (keyword tier is already wired).
- Bundling `skills/` and `mcp-servers/` into the Tauri installer so releases
  are fully self-contained without the source tree.

## Star History

<a href="https://star-history.com/#xuzhougeng/wisp-science&Date">
  <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=xuzhougeng/wisp-science&type=Date" />
</a>
