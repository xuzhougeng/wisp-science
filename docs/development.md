# Development

Build, architecture, CLI environment, and tests. For first-run desktop setup see
[basic configuration](basic-configuration.md). For HTTP model profiles see
[model configuration](model-configuration.md).

## GitHub Pages tutorials

The website's [tutorial directory](tutorials.html) links to one independent page
per article under `docs/tutorials/`, generated from `docs/wechat/*.md` and matching
English translations in `docs/wechat/en/`. The directory contains only compact
cards. The language switch changes titles, complete article text, captions,
examples, source links, and previous/next navigation. Each language uses screenshots
of the corresponding app interface; English assets live under
`docs/assets/tutorials/en/`. Article links open the corresponding tutorial
page; other Markdown documentation links open the repository source. Each article
offers a return link at the top and bottom and previous/next navigation. Returning
to the directory restores the matching card via its stable anchor.

After editing or adding an article, regenerate the checked-in page:

```bash
python3 -m pip install -r docs/requirements-pages.txt
python3 docs/build_tutorials.py
python3 docs/build_tutorials.py --check
python3 -m unittest discover -s docs -p 'test_build_tutorials.py'
```

Each article must start with a level-one title. `READING_ORDER` in the generator
puts Quick Start first, then the four introductory tutorials, MCP, Skills, trajectories,
and the advanced CLI and ACP tutorials; other articles are appended alphabetically. Filenames determine
stable article URLs and directory card anchors. Edit the page shell outside the generated
markers in `docs/tutorials.html`; edit both language sources when updating articles.
The generator rejects missing English translations instead of silently showing
Chinese content in English mode. Relative links in English sources use their own
`en/` directory as the base (for example, `../../assets/` for screenshots).
The Pages workflow regenerates both the directory and article pages before
uploading them. Do not hand-edit the generated files under `docs/tutorials/`.

For a manual smoke check, serve `docs` with `python3 -m http.server --directory docs
8080`. Open the homepage, follow Tutorials in the header or footer, and check all
directory links, the Skills code example, and the tutorial screenshots. Each
card should open only its own article. Check both return links, previous/next
navigation, browser Back, and refreshing an article's direct URL.
Repeat at a narrow mobile width and switch to English; the surrounding navigation
and all article text should switch language. Reload a direct `?lang=en` URL,
follow another tutorial, and return to the directory; English must persist even
when browser storage is unavailable. The language is carried in local HTML links.

The homepage links directly to Quick Start. Models and ACP are accessed through
the tutorial directory; the former standalone configuration HTML pages and their
navigation entries have been removed without redirect pages.

The tutorial screenshots use the real frontend with localized mock data. The
English set also covers ACP, the trajectory inspector, and the actual Chromium
extension manager in a fresh offline profile. Regenerate them without API keys,
real servers, or live website access:

```bash
cd ui-tests
WISP_TUTORIAL_SHOTS=../docs/assets/tutorials npx playwright test tests/tutorial-screenshots.spec.ts
# Only regenerate the English assets:
WISP_TUTORIAL_LOCALE=en WISP_TUTORIAL_SHOTS=../docs/assets/tutorials npx playwright test tests/tutorial-screenshots.spec.ts
```

The capture tests normally write to Playwright's output directory; only the
explicit environment variable updates the published assets. Their demo account,
host, terminal output, and conversation data are labeled as examples in the
articles. Quick Start captures all four onboarding steps, project creation, and a
first conversation with a deterministic mock reply; it never calls a model API.
Chinese assets retain their existing paths; English captures go into `en/`.
The English screenshot tests reject visible Chinese text, including input values.
The Chrome/Chromium installation screenshot uses Playwright's full Chromium
binary, included by `npx playwright install chromium`, rather than headless shell.

The [Skills catalog](skills.html) groups all bundled Skills into research tasks.
Reviewed Chinese and English summaries live in `docs/skills-catalog.json`.
`docs/build_skills.py` checks the catalog against `skills/*/SKILL.md` before
generating the page; adding or removing a bundled Skill also requires updating
its catalog entry. To regenerate and verify it:

```bash
python3 docs/build_skills.py
python3 docs/build_skills.py --check
python3 -m unittest discover -s docs -p 'test_build_*.py'
```

Website copy lives in `docs/assets/i18n.js`; keep the static Chinese fallback in
the homepage and MCP page consistent with it. Research-task cards are illustrative
requests, not attributed testimonials. The privacy FAQ distinguishes local
storage from content sent to configured model or data services.

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

The desktop icon uses the three-wisp design by
[SpicyChicken6 in Discussion #1154](https://github.com/xuzhougeng/wisp-science/discussions/1154),
with a teal mark (`#0D9488`) on an off-white tile (`#FAF9F6`). The original
arc paths and optical centering are preserved from the
[editable SVG bundle](https://raw.githubusercontent.com/SpicyChicken6/wisp-science/a1b35d00f2031890d835eefa6196cbb25fce058b/logo-proposal/wisp-science-logo-and-icon-assets.zip).

Regenerate the checked-in assets with `pwsh -File src-tauri/gen-icons.ps1`
(or `powershell -ExecutionPolicy Bypass -File src-tauri/gen-icons.ps1` on Windows).
This uses `cargo tauri icon` and three 1024px SVG masters:

- `src-tauri/icons/app-icon.svg`: full square for store/mobile assets and
  `source.png`.
- `src-tauri/icons/app-icon-rounded.svg`: rounded tile for Windows ICO and
  desktop PNGs, including Linux launchers and the tray.
- `src-tauri/icons/app-icon-macos.svg`: rounded tile with a 10% transparent
  margin on each side for the bundled macOS ICNS. Legacy ICNS needs this
  geometry baked into the bitmap; it is not a layered Icon Composer asset.

macOS 26+ uses the native `Wisp.icon` Icon Composer document, with the same
light palette and the author's dark palette (`#2DA898` on `#171614`, from
`app-icon-dark.svg`). The system's icon appearance preference selects the
variant in both Dock and Finder, including when the app is closed. This is
independent of Wisp's in-app theme setting. Older macOS versions, Windows,
and Linux use the light icon.

To follow system appearance, choose **System Settings → Appearance → Icon &
widget style → Dark → Auto**. **Default** keeps the light icon; **Dark →
Always** keeps the dark icon. See [Apple's appearance settings guide](https://support.apple.com/guide/mac-help/change-appearance-settings-mchlp1225/mac).

After editing either palette, run `python3 src-tauri/gen-icons-macos.py` on
macOS with Xcode 26+ selected. It updates the transparent SVG layers, compiles
`icons/Assets.car`, checks for both native appearances, and records source
and output hashes in `icons/macos-icon-build.json`. Commit those outputs
together. The compiled catalog is checked in so normal builds on other
platforms or older Xcode hosts do not need Icon Composer. The macOS bundle
places it at `Contents/Resources/Assets.car`; `Info.plist` selects `Wisp` via
`CFBundleIconName`, while the existing `icon.icns` remains the legacy fallback.
Native macOS rendering may add system material and lighting to the design.

The in-app and website logos are separate assets. Do not generate desktop
icons from `ui/logo.svg`. The current mark is the three-wisp symbol; the
previous helix is kept as `docs/assets/logo_v1.svg` and `ui/logo_v1.svg`.

The full molecular wordmark also comes from **SpicyChicken6** in the same
[Discussion #1154](https://github.com/xuzhougeng/wisp-science/discussions/1154)
and editable SVG bundle. Thank you for both the wordmark and the desktop
icon designs. `docs/assets/wordmark-light.svg` and `wordmark-dark.svg` retain
the original paths and colors, with only the unused canvas trimmed. Both
are transparent and all lettering is outlined, so they need no font files.

The README files select these assets using a theme-aware `<picture>`.
The website hero uses the light wordmark on its existing light background.
Trunk copies the same SVGs into the app bundle for the projects home and
empty chat welcome area. **Wisp Settings → Appearance → System / Light /
Dark** selects the in-app variant on every platform; System follows OS
appearance. This setting is separate from the native macOS desktop icon
appearance described above. When editing a wordmark, check both app surfaces
in all three theme modes, including an app theme opposite to the OS theme,
and check that the projects toolbar still fits a narrow window.

For a manual smoke check, build/install on each target OS and inspect the
Dock/Finder (macOS), taskbar/shortcut (Windows), or launcher (Linux). Check
that the three wisps remain legible at small sizes, corners are transparent,
and the macOS tile has a comparable footprint to neighboring Dock icons.
On macOS 26+, select light and dark system icon appearances and check both
Dock and Finder, including with the app closed. `cargo tauri dev` is not a
packaged application and does not exercise the native asset catalog.

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

Startup, session initialization, and CLI startup never create Python virtualenvs
or install dependencies. The desktop checks executable paths in the background
and shows the results during first-run model setup, Settings → General → Local environment, and
Capabilities. Missing Python/R/uv/Node tools are optional setup suggestions,
not startup errors. Found paths are persisted on the Local execution context;
existing interpreter overrides survive detection and database reopen. The
check does not launch executables or validate versions/packages. **Cmd/Ctrl+P →
Quick setup** reopens the first-run page and refreshes detection
without resetting models or installing software. Use **Check
paths again** after installing tools. Windows Store Python aliases and the
macOS system Python developer-tools stub are excluded from automatic selection.

For analysis or custom Python MCP servers, ask Wisp to load
`skills/local-env-setup`. The skill prepares only the required environment and
saves Python/R interpreters through `set_runtime_interpreter` or the existing
Runtime interpreters dialog. The CLI uses its existing `.wisp/python/.venv`
when present, otherwise Python on PATH. Already-running REPLs retain their
interpreter until restarted.


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
still overrides the bundled tools entirely. External Python MCP servers need
their own environment, prepared through `local-env-setup`.

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
- [bear-research-skills](https://github.com/fei0810/bear-research-skills)
  is available as an optional Skills marketplace source, rather than bundled
  files. Its CC BY-NC-SA 4.0 license and `scimaster-cli` dependency apply when installed.
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
