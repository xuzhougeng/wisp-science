---
name: local-env-setup
description: Prepare optional local Python/R runtimes, user-configured Python MCP servers, Node/scimaster-cli, and pixi for the user’s task. Reuse detected paths, configure Wisp interpreters, and apply mirrors when needed. Use for 配置环境, missing runtime dependencies, or requested local installations. For remote SSH compute use compute-env-setup.
license: Apache-2.0
tags: environment, uv, python, r, node, npm, pixi, scimaster, mirror, china, install, macos, windows, linux
---

# Local runtime setup

Wisp starts and runs native biological tools without Python, R, uv, or Node.
First-run model setup and Settings → Models show a background path check.
**Cmd/Ctrl+P → Quick setup (快速配置)** reopens setup and refreshes the check.
It never runs installers or creates a virtualenv. Detected executable paths are
saved in the Local execution context without replacing existing choices.
A found path does not establish version compatibility or installed packages.

| Task | Optional tools |
|---|---|
| Python analysis / persistent `python` tool | Existing Python environment; uv can help create one |
| R analysis / persistent `r` tool | Rscript with `jsonlite` |
| User-configured Python MCP | That server's own interpreter and dependencies |
| `bear-*` literature skills | Node >= 20, npm, sci (scimaster-cli) |
| Bioinformatics workflows | pixi for isolated conda/pip environments |

Start with the user's task and existing environment. Install only what it
needs. Do not treat every missing optional tool as a broken installation.
After changing PATH, use **Check paths again** in model settings; restart Wisp
only if the GUI process needs to inherit a changed PATH. Interpreter settings
can be saved directly without restarting the app.

## Step 0 — Detect platform, region, and current state

Read the **Environment** section in the system prompt (`Operating system`, `Working directory`).

### 0a — Region / network (mirror or not)

**Before any install or `pip`/`npm`/`pixi add`, decide whether the user is on mainland China and needs mirrors.**

Signals (use several; do not rely on one):

| Signal | Mainland likely |
|---|---|
| User writes in Chinese and mentions 国内 / 镜像 / 翻墙 / 清华 / 阿里 | yes |
| `TZ` / system timezone `Asia/Shanghai`, `Asia/Chongqing`, `Asia/Urumqi` | hint |
| Locale `zh_CN`, `zh-Hans-CN` | hint |
| `curl -s --connect-timeout 3 https://pypi.org/simple/` fails or >5s; tuna mirror responds in <2s | yes |
| User explicitly says they are **not** in China / have full international access | no |

If **ambiguous**, ask once: "Are you on mainland China? I'll use domestic mirrors for pip/npm/conda if yes."

When **mainland mirrors apply**, set these **before** installs (user shell profile or session env):

```sh
# PyPI / uv (Python environments + pixi pip deps)
export UV_INDEX_URL=https://pypi.tuna.tsinghua.edu.cn/simple
export PIP_INDEX_URL=https://pypi.tuna.tsinghua.edu.cn/simple

# npm (scimaster-cli)
npm config set registry https://registry.npmmirror.com
```

Windows (PowerShell, persist for user):

```powershell
[Environment]::SetEnvironmentVariable("UV_INDEX_URL", "https://pypi.tuna.tsinghua.edu.cn/simple", "User")
[Environment]::SetEnvironmentVariable("PIP_INDEX_URL", "https://pypi.tuna.tsinghua.edu.cn/simple", "User")
npm config set registry https://registry.npmmirror.com
```

**Pixi conda channels** (global or per-project `pixi.toml`):

```toml
[project]
channels = ["https://mirrors.tuna.tsinghua.edu.cn/anaconda/cloud/conda-forge/"]

[pypi-config]
index-url = "https://pypi.tuna.tsinghua.edu.cn/simple"
```

Or global:

```sh
pixi config set --global pypi-config.index-url https://pypi.tuna.tsinghua.edu.cn/simple
```

Alternatives if tuna is slow: Aliyun PyPI `https://mirrors.aliyun.com/pypi/simple/`, USTC conda mirrors.

If international access works, **do not** set mirrors — use defaults.

### 0b — Tool presence

Run with **`shell`** (PowerShell on Windows, `sh -c` elsewhere):

**Windows:**

```powershell
Get-Command python,python3,Rscript,uv,node,npm,sci,pixi -ErrorAction SilentlyContinue | Select-Object Name,Source
uv --version 2>$null; node --version 2>$null; npm --version 2>$null; sci --version 2>$null; pixi --version 2>$null
```

**macOS / Linux:**

```sh
for c in python3 python Rscript uv node npm sci pixi; do command -v $c && $c --version 2>/dev/null; done
```

The optional environment panel in onboarding, model settings, and Capabilities shows detected paths. Inspect the saved Local interpreter settings before choosing another environment.

## Python and R, when requested

Reuse an existing project environment when suitable. In the desktop app,
`set_runtime_interpreter` saves a path on the execution context:

```json
{"context_id":"local","language":"python","executable":"/absolute/path/to/python"}
```

For R, use `"language":"r"` and an absolute `Rscript` path. Users can also
open **Runtime interpreters** and browse for either executable. Running REPLs
keep their previous interpreter until restarted; the agent tool restarts the
current conversation's matching REPL and clears its variables.

For Python, invoke the chosen executable to verify its version and install only
needed packages. `python/requirements-kernel.txt` lists common analysis
packages shipped as a requirements file; native biological tools require none
of them. The kernel worker itself uses the Python standard library.
For R, verify `Rscript -e 'library(jsonlite)'` using the chosen absolute path;
install `jsonlite` in that environment if needed, then test the Wisp `r` tool.

The CLI uses an existing environment at `<workspace>/.wisp/python/.venv` if
present, otherwise Python on PATH. The desktop retains these legacy fallback
locations for users who already prepared them; it never creates them on launch:

| OS | Desktop venv path |
|---|---|
| Windows | `%APPDATA%\science.wisp-science\wisp-science\python\.venv` |
| macOS | `~/Library/Application Support/science.wisp-science/wisp-science/python/.venv` |
| Linux | `~/.local/share/science.wisp-science/wisp-science/python/.venv` |

### Install uv

**International:**

```powershell
# Windows
powershell -ExecutionPolicy Bypass -c "irm https://astral.sh/uv/install.ps1 | iex"
```

```sh
# macOS / Linux
curl -LsSf https://astral.sh/uv/install.sh | sh
```

**Mainland China:** prefer **winget** / **Homebrew** / distro package if the astral installer is slow or blocked; set `UV_INDEX_URL` (above) before `uv pip install`.

```powershell
winget install --id astral-sh.uv -e          # Windows
```

```sh
brew install uv                               # macOS
```

Default binary: `~/.local/bin/uv` (Unix) or `%USERPROFILE%\.local\bin\uv.exe` (Windows). Ensure that dir is on PATH.

### Python via uv

```sh
uv python install 3.11
uv python list
```

Target: **Python 3.11+**. With mainland mirrors, export `UV_INDEX_URL` first.

### Create a project environment when needed

Set `ENV_DIR` to a task-specific environment location and `REQ` to the bundled
or repository `python/requirements-kernel.txt` when those analysis packages
are wanted. Reuse an existing suitable environment instead of recreating it.

```sh
uv venv "$ENV_DIR"
uv pip install -r "$REQ" --python "$ENV_DIR/bin/python"
"$ENV_DIR/bin/python" -c "import sys; print(sys.executable)"
```

Windows PowerShell: use `$envDir`, `$req`, and `Scripts\python.exe`:

```powershell
uv venv $envDir
uv pip install -r $req --python "$envDir\Scripts\python.exe"
& "$envDir\Scripts\python.exe" -c "import sys; print(sys.executable)"
```

Save that absolute interpreter through `set_runtime_interpreter` when available,
then verify a small `python` tool call. If the tool is unavailable in the CLI,
use the CLI fallback location above or launch with the prepared environment on PATH.

### User-configured Python MCP servers

Read that server's installation requirements. Prepare its own environment and
configure its MCP command with the absolute executable and appropriate arguments.
Do not reinstall the removed `mcp-servers/bio-tools` tree or add Python MCP
packages merely to use Wisp's native biological tools. Test the configured
connection after setup.

## Layer 2 — Literature: Node + scimaster-cli

Required for bundled **`bear-support`**, **`bear-counter`**, **`bear-map`**, **`bear-scoop`**, **`bear-trace`**, **`bear-review`**, **`bear-onboard`**, **`bear-propose`**.

### Install Node >= 20

**International:** https://nodejs.org/ LTS, or `winget install OpenJS.NodeJS.LTS`, or `brew install node`.

**Mainland China:**

```powershell
# Windows — winget often works; or npmmirror-hosted installer
winget install OpenJS.NodeJS.LTS
```

```sh
# macOS — brew or fnm with npmmirror
brew install node
# fnm alternative:
# export FNM_NODE_DIST_MIRROR=https://npmmirror.com/mirrors/node
# fnm install 20 && fnm use 20
```

After install, open a **new** terminal; verify `node --version` (v20+).

### scimaster-cli

Set npm registry first if mainland (see 0a), then:

```sh
npm install -g scimaster-cli
sci init        # paste SciMaster API Key
sci --version
sci usage
```

API Key: SciMaster settings → API Key. Do **not** proceed with bear-* skills if `sci --version` fails.

In the wisp-science desktop app, you can also save the SciMaster key in
Settings -> Credentials -> SCIMaster. Wisp will sync that key into
`~/.scimaster/config.json` for `scimaster-cli`.

## Layer 3 — Bioinformatics: pixi

**pixi** manages isolated per-project environments (conda + pip) — use for scanpy/single-cell, variant calling stacks, etc. The Wisp **`python` tool** uses the selected context interpreter. Run project bioinformatics commands via **`shell`**: `pixi run python …` or `pixi run …` in the project directory.

### Workflow engines

For **multi-step analysis pipelines** (many rules, parallel execution, resume after failure), pair pixi with a dedicated workflow engine — pixi manages the environments, the engine schedules the steps. Run engines from **`shell`** in the project directory.

| Engine | What it is | When to choose |
|---|---|---|
| [snakemake](https://snakemake.github.io) | Python-defined rules, mature conda/mamba integration | Established rules; Python-centric teams |
| [nextflow](https://www.nextflow.io) | Groovy DSL, container-first | nf-core ecosystem; HPC/cloud portability |
| [oxo-flow](https://github.com/Traitome/oxo-flow) | Rust-native, TOML-defined DAG engine; CLI + web UI | Lightweight single-binary install; rule-level conda/mamba/pixi/docker/singularity backends; checkpoint/resume |

### Install pixi

**International:**

```sh
curl -fsSL https://pixi.sh/install.sh | bash
```

```powershell
powershell -ExecutionPolicy ByPass -c "irm -useb https://pixi.sh/install.ps1 | iex"
```

**Mainland China:** if install script is slow, try `brew install pixi` (macOS) or download release from GitHub mirror; then configure mirrors (0a).

### Typical project workflow

In the user's analysis directory:

```sh
pixi init
pixi add scanpy anndata          # example; adjust to task
pixi run python analysis.py
```

Multiple envs: use `[environments]` / features in `pixi.toml`, or separate project dirs — see [pixi docs](https://pixi.sh).

With mainland mirrors, set `[pypi-config]` and `channels` in `pixi.toml` (0a) **before** large `pixi add`.

### Verify pixi

```sh
pixi --version
pixi info    # shows config paths and channels
```

## Workarounds

| Issue | Fix |
|---|---|
| uv/node installed but app still says missing | Restart wisp-science; confirm tools on PATH for the **GUI user** (macOS: relaunch from Dock after shell profile update). |
| Cannot modify PATH | Set `UV_PATH` / `PIXI_PATH` to full binary paths before launching wisp-science. |
| Mainland: timeouts on pypi.org / registry.npmjs.org | Apply Step 0a mirrors; retry. |
| Corporate proxy / TLS | `HTTPS_PROXY`, trust store; still use mirrors if direct egress to US is blocked. |
| Broken Python environment | Diagnose or create a replacement environment, install needed packages, then save its interpreter. Restarting Wisp does not repair or recreate environments. |
| bear-* skill stops at CLI check | Install Node + `scimaster-cli` + `sci init`; do not fake citations. |

## Agent workflow

1. Load this skill for requested setup or a task blocked by a missing dependency.
2. Detect OS and existing paths; reuse the user's selected environment.
3. Before downloading, choose appropriate network mirrors (Step 0a).
4. Install only the Python/R, custom MCP, literature, or bioinformatics components needed.
5. Save interpreter paths to the chosen execution context and verify actual task/tool execution.
6. Report installed components, selected paths, and any task-specific dependencies still missing.

## Not in scope

- Remote GPU or direct SSH → `compute-env-setup`; managed cloud backends are
  unavailable until Wisp implements a matching execution-context backend
- Replacing pixi with conda/micromamba when pixi suffices locally
- SciMaster API billing / key provisioning beyond pointing to `sci init`
