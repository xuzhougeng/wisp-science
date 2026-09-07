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

Desktop icons come from two masters via `src-tauri/gen-icons.ps1`
(`cargo tauri icon`). Both keep the DNA mark inset; do not regenerate from
`ui/logo.svg`, which fills the canvas and looks oversized on every launcher.

- `src-tauri/icons/app-icon.svg` is full-bleed. macOS Dock/Launchpad apply a
  squircle, so a baked badge would be double-masked and look small.
- `src-tauri/icons/app-icon-rounded.svg` clips the same mark to rounded
  corners. Windows taskbar and most Linux docks draw the bitmap as-is.

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
| `WISP_VISION`        | `1`/`true` if the primary model can read images natively (default off) |
| `WISP_VISION_MODEL`  | Dedicated image-analysis model when the primary model cannot see |
| `WISP_VISION_PROVIDER` | Vision provider kind; defaults to `WISP_PROVIDER`           |
| `WISP_VISION_API_URL` | Vision API root; defaults to `WISP_API_URL` or the provider default |
| `WISP_VISION_API_KEY` | Optional vision key; defaults to `WISP_API_KEY`             |
| `WISP_MAX_CONTEXT`   | Context budget (default 1,000,000)                            |
| `WISP_MAX_ITER`      | Max agent iterations per turn (default 100; 0 = unlimited)    |
| `WISP_SKILLS_PATH`   | Extra `;`/`:`-separated SKILL.md catalog dirs                 |
| `WISP_KERNEL_WORKER` | Override path to `kernel_worker.py` (bundled by default)      |
| `WISP_MCP_COMMAND`   | Launch an arbitrary stdio MCP server (full command line)      |
| `WISP_MCP_PKG`       | Select a native bio package, e.g. `mcp_pubmed`          |

Desktop stores API keys in the OS keyring and model profiles in
`.wisp/wisp.sqlite`. Custom credentials map a display name to an environment
variable and are injected only into newly launched local Python and bundled MCP
processes — never copied to SSH/WSL hosts.

Wisp reads `AGENTS.md` from the project root when a new session starts.
Instructions in **Project Settings → Agent Context** live in `.wisp/WISP.md`
and take precedence when both exist.

### Native biological tools and custom MCP

Native biological retrieval lives in `crates/wisp-bio/`. `NativeBio` is the
shared client (HTTP + 设置→凭据 / CLI env). Each domain is a module with
`catalog()` and `call()`; the native catalog is the only built-in inventory.
`mcp_bio` selects every implemented domain; `WISP_MCP_PKG=mcp_<domain>` selects
one. New upstream API keys belong in **设置 → 凭据** (`src-tauri/src/models.rs`
`CREDENTIALS`) and are read with `NativeBio::credential`.

The PubMed domain is complete. All seven PubMed operations
(`search_articles`, `get_article_metadata`, `convert_article_ids`,
`find_related_articles`, `lookup_article_by_citation`, `get_full_text_article`,
`get_copyright_status`) use independently authored Rust clients in the desktop,
CLI and ACP MCP bridge. They keep deferred tool discovery and PubMed connector
controls. Search and metadata use NCBI E-utilities; identifier conversion uses
the PMC ID Converter; related records use ELink; citation lookup uses ECitMatch;
OA full text and copyright/access metadata use Europe PMC core plus converter
embargo fields. The retired PMC OA Web Service is not called. An open-access
flag is not treated as a reuse grant.

All 23 domains (247 tools) run in Rust, including KEGG, CADD, PanglaoDB and
Sanger Cell Model Passports. `mcp-servers/bio-tools` and its launcher, copied
schemas and Tauri resource mapping have been removed. Native tool discovery,
connector settings and delegated grants do not require a Python environment.
Python/R remain available for scientific computation; their runtime setup is
independent of biological retrieval. Python requirements now live in
`python/requirements-kernel.txt`; the MCP server package is no longer installed.

`WISP_MCP_COMMAND` still replaces the built-in tools with an explicit external
stdio server; custom stdio/HTTP MCP connections remain supported. Unknown
`WISP_MCP_PKG` values produce a configuration diagnostic. NCBI contact/key values
continue to come from desktop keyring settings or CLI environment variables.
No credentials are copied to SQLite or bundled with the native clients.

Important data-contract changes after live acceptance:

- BioMart uses the dedicated mart host; query POSTs rejected with 405 retry the
  documented GET XML form. Ensembl REST explicitly requests JSON.
- bioRxiv statistics accept object-valued status messages. PubMed citation
  lookup requests `retmode=xml`. Reactome tokens are decoded and safely encoded
  as URL path segments instead of rejecting encoded base64 padding.
- cBioPortal totals are null when no total header is supplied. PubChem CID-only
  placeholders are reported as missing records.
- eQTL listing reads the official metadata once per process. Associations use
  tabix indexes and bounded HTTPS ranges of the official summary files, not the
  retired REST API. `pos` is a GRCh38 interval; gene-only queries use the current
  Ensembl TSS ±1 Mb and report that scope. Results may be truncated by the row,
  compressed-byte or scan limit. A dataset is never downloaded in full.
- Rfam sequence search uploads a small multipart `sequence_file` to
  `batch.rfam.org/submit-job` and waits for a completed result, including
  JSON progress responses returned with HTTP 200.
- ZINC random sampling uses its dedicated random-job poll URL. Supplier codes
  are case-sensitive and may occur inside catalog rows. SMILES searches use the
  enabled public SmallWorld ZINC20 for-sale index and report its name; this is
  not exhaustive ZINC22 coverage. ID and 3D-tranche lookups still support ZINC22.

See the [native migration design](superpowers/specs/2026-09-06-native-bio-services-design.md)
for architecture and implementation provenance.

`WISP_MCP_PKG=mcp_pubmed` (or any `mcp_<domain>`) selects the native catalog for
that package. `mcp_bio` selects every implemented domain. `WISP_MCP_COMMAND`
still overrides the bundled tools entirely. The vendored launcher remains in
tree until the removal slice. Historical Python install notes:

```bash
uv pip install mcp requests
# plus any server-specific deps (httpx, xmltodict, etc.) the package imports
```

The agent discovers matching tools with `search_mcp_tools` and calls the
selected one through `use_mcp_tool`; the full server catalog is never copied
into every model request.

Desktop users add remote MCP (Notion, Parallel Search, …) under
**Settings → Connections**. Connector detail pages show introductions, expandable
tool descriptions and input schemas, plus source/documentation links. Native
introductions are maintained in `crates/wisp-bio/src/domains.json`; their domain
coverage is checked against `catalog()`. Tool descriptions and schemas come
from that same dispatch catalog. Custom MCP tools retain the server's description,
`inputSchema` and optional `outputSchema`; absent metadata is left absent.
Connector DTOs are shared in `wisp-dto`. Browsing documentation neither invokes
a native retrieval tool nor changes approvals. See [basic configuration](basic-configuration.md).

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
│  ├─ wisp-mcp/     stdio/HTTP MCP client + McpTool adapter (custom servers)
│  ├─ wisp-bio/     Native biological database clients shared by all hosts
│  ├─ wisp-acp/     ACP v1 stdio client for external coding agents
│  ├─ wisp-sync/    Encrypted snapshot protocol + self-hosted relay server
│  ├─ wisp-runs/    Run control plane (run_in_context / monitor_run / harvest)
│  └─ wisp-cli/     `wisp-science` headless binary
├─ src-tauri/       Tauri v2 desktop shell (commands + agent event stream)
├─ ui/              Leptos CSR frontend (built by Trunk, loaded in WebView2)
├─ python/          kernel_worker.py + mock MCP server (uv-managed)
├─ r/               optional system-R kernel worker (requires jsonlite)
├─ skills/          Bundled SKILL.md catalog for reusable scientific workflows
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
  dangerous-command gating. Relative filesystem paths resolve from the active
  project root; isolated exploration and delegated sessions additionally keep
  reads and searches inside that root.
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
- `skills/` vendored from the upstream
  `wisp-science` asset bundle (Apache-2.0).
- `skills/bear-*` from [bear-research-skills](https://github.com/fei0810/bear-research-skills)
  (CC BY-NC-SA 4.0); requires `scimaster-cli` for live retrieval.
- `python/kernel_worker.py` protocol adapted from the upstream operon kernel
  worker, with POSIX-only `resource`/`/proc`/`SIGINT` machinery dropped for
  Windows.
- `docs/assets/trusted-logos/meduniwien.svg` from
  [Wikimedia Commons](https://commons.wikimedia.org/wiki/File:Meduni-wien.svg)
  (public domain; the mark itself is trademarked), cropped to the circular
  emblem.
- Institution marks in `docs/assets/trusted-logos/` (Cornell, Michigan, UCLA,
  Yale, Tsinghua, Zhejiang, WashU, SLU, SJTU, PKU, CAS) match the set served by
  [wispscience.com](https://wispscience.com/institutions/), plus Medical
  University of Vienna. The marks themselves remain trademarked by their owners.
