# wisp-science

A Windows- and macOS-ready, open-source scientific computing agent — a Rust rewrite of
the Claude Science (Operon) concept, with the agent core ported from
[`m4n9H/mangopi-cli`](https://github.com/w4n9H/mangopi-cli) and the biology
tooling vendored from the upstream `wisp-science` asset bundle.

wisp-science is a local-first desktop copilot for science: it talks to any
OpenAI-compatible or Anthropic model, runs a persistent Python REPL, calls
tools on the local filesystem, loads reusable `SKILL.md` workflows, and
reaches ~80 biological databases through bundled MCP servers — all from a
Tauri v2 desktop window (WebView2 on Windows, system WebKit on macOS) or a
headless CLI.

> Status: MVP vertical slice. The agent loop, streaming providers, tools,
> Python REPL, SQLite store, MCP client, and Leptos UI all build and run.
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
│  ├─ wisp-python/  uv venv provisioning + Windows kernel_worker + `python` REPL tool
│  ├─ wisp-mcp/     stdio JSON-RPC MCP client + McpTool adapter (bundled bio-tools)
│  └─ wisp-cli/     `wisp-science` headless binary
├─ src-tauri/       Tauri v2 desktop shell (commands + agent event stream)
├─ ui/              Leptos CSR frontend (built by Trunk, loaded in WebView2)
├─ python/          kernel_worker.py + mock MCP server (uv-managed)
├─ skills/          Bundled SKILL.md catalog (29 science workflows)
├─ mcp-servers/     Bundled MCP servers (bio-tools: ~80 DB clients)
└─ seed/            Bundled demo session recordings (CRISPR / enzyme / extremophile / immunotherapy)
```

> The upstream `drizzle/` (TypeScript ORM migrations) is **not** used —
> `wisp-store` ships its own `sqlx` SQLite schema
> (`crates/wisp-store/migrations/0000_init.sql`). The desktop app persists
> conversations, settings, and artifacts there; API keys go to the OS keyring.

## Prerequisites

- **Rust** (stable, 1.80+) with `wasm32-unknown-unknown`:
  `rustup target add wasm32-unknown-unknown`
- **uv** (Python environment manager): <https://docs.astral.sh/uv/>
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
REPL (provisioning a uv venv at `.wisp/python/.venv` on first run).

### Desktop app

```powershell
cargo tauri dev      # hot-reload: Trunk serves UI, Tauri opens WebView2
cargo tauri build    # produce an MSI/NSIS installer under target/release/bundle
```

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
providers or local Codex CLI / Claude Code runners. See
[Model configuration](docs/model-configuration.md) for the provider fields and
local runner notes. **Conversations persist to that SQLite database** — each
turn's messages are appended to the active session frame, so restarting the app
restores the full history. The headless CLI keeps using `.wisp/session.json`
for portability.

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
  npx playwright test                  # serve UI + run 3 flows (demo / send / settings)
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
- **Python REPL** (`wisp-python`): a long-lived `kernel_worker.py` subprocess
  keeps a persistent namespace across cells; `stdout_chunk` lines stream live
  to the UI.
- **MCP** (`wisp-mcp`): a minimal newline-JSON-RPC client launches any stdio
  MCP server and exposes each remote tool as a first-class agent tool.

## Attribution

- Agent core ported from `w4n9H/mangopi-cli` (Apache-2.0).
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
- `loop_engine` — Implementer / Verifier / Updater multi-agent loop (the
  upstream REVIEWER concept).
- Artifact management + inline Mol* 3D structure viewer in the UI.
- `RoutedProvider` LLM-score tier selection (keyword tier is already wired).
- Bundling `skills/` and `mcp-servers/` into the Tauri installer so releases
  are fully self-contained without the source tree.
- R kernel support (uv is Python-only; system R for now).

### Claude Science UX parity

Feature-design targets drawn from the upstream Claude Science walkthrough. Each
line notes what wisp ships today versus the reference behaviour.

- **Multi-step onboarding.** Reference: *Connect to the scientific web*
  (per-category database toggles) → *Connectors & skills* (toggle each
  connector/skill) → *What do you work on?* (free-text profile that seeds
  suggested starter tasks). wisp today: a minimal welcome + API-key step
  (`get_onboarding_state` / `dismiss_onboarding`).
- **Connectors panel.** Per-category on/off switches over the ~80 bundled MCP
  bio-tools (Cancer Models, CellGuide, Clinical Genomics, Expression, Genomes,
  Human Genetics, Literature Graph, Protein Annotation, …) instead of the
  current env-var launch (`WISP_MCP_PKG`). Pairs with a **Network →
  allowed-domains** allowlist gating agent web access.
- **Workspace settings sections.** Reference groups config into Skills,
  Connectors, Specialists, Memory, Compute, Network, Permissions, Credentials,
  Storage, Logs, General. wisp today: a Settings modal (provider/key) plus a
  read-only Capabilities view.
- **Inline tool-approval card.** Render the approval prompt as a card in the
  conversation flow ("Run Python code? · Allow for this conversation · Deny")
  rather than the current centered confirm modal. The `confirm-request`
  plumbing already exists — this is a presentation change.
- **Artifacts gallery.** Thumbnail grid for figure artifacts (PNG/plots),
  plus figure↔caption pairing (a plot alongside a structured caption doc:
  *Panels / Artifacts / what is real vs. illustrative*). wisp today: a text
  tile list with a single active preview.
- **Projects home.** Multiple projects, each with session/artifact counts and
  a "+ New project" action, versus today's single project + flat session list.
- **Web-search toggle** surfaced directly in the composer.

The reference is a general computational-research environment (the walkthrough
also runs an economics/tariff pass-through analysis), consistent with wisp's
provider- and domain-agnostic core — the science tooling is bundled, not
hard-wired.
