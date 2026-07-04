---
name: local-env-setup
description: Configure the local wisp-science runtime — uv/Python bootstrap, Node+scimaster-cli for bear-* literature skills, pixi for bioinformatics multi-env analysis. Detect mainland-China network and apply mirrors. Use when Capabilities shows missing Python/uv/Node/sci/pixi, bootstrap errors, or the user asks to 配置环境 / install Python / uv / Node / pixi / set up the local environment. Not for remote GPU/SSH compute (use compute-env-setup).
license: Apache-2.0
tags: bootstrap, uv, python, node, npm, pixi, scimaster, mirror, china, install, macos, windows, linux
---

# Local runtime setup

wisp-science needs three **independent** local toolchains:

| Layer | Tools | Purpose |
|---|---|---|
| **Core** | `uv` + managed Python venv | App bootstrap, `python` tool, bundled MCP servers |
| **Literature** | Node >= 20, `npm`, `sci` (scimaster-cli) | Bundled `bear-*` skills (real paper search) |
| **Bioinformatics** | `pixi` | Per-project conda/pip multi-env analysis (scanpy, nextflow-adjacent stacks, etc.) |

Core is **required** for the app. Literature and bioinformatics layers are optional until the user runs those skills — but Capabilities shows all of them; install what's missing for the user's goal.

Restart wisp-science after changing PATH or global config so bootstrap re-runs.

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
# PyPI / uv (core bootstrap + pixi pip deps)
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
Get-Command uv,node,npm,sci,pixi -ErrorAction SilentlyContinue | Select-Object Name,Source
uv --version 2>$null; node --version 2>$null; npm --version 2>$null; sci --version 2>$null; pixi --version 2>$null
```

**macOS / Linux:**

```sh
for c in uv node npm sci pixi; do command -v $c && $c --version 2>/dev/null; done
```

**Capabilities** (能力) shows: `Python · uv · Node · sci · pixi · skills · MCP`.

## Layer 1 — Core: uv + Python

wisp-science does **not** ship Python. It needs **`uv`** on PATH (or `UV_PATH`) to create the managed venv.

### What gets created automatically

1. `uv venv` → virtualenv under app data
2. `uv pip install -r …/python/requirements-mcp.txt`
3. Marker `.wisp_deps_ok` when deps succeed

| OS | Desktop venv path |
|---|---|
| Windows | `%APPDATA%\science.wisp-science\wisp-science\python\.venv` |
| macOS | `~/Library/Application Support/science.wisp-science/wisp-science/python/.venv` |
| Linux | `~/.local/share/science.wisp-science/wisp-science/python/.venv` |

Dev checkout: `<workspace>/.wisp/python/.venv`

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

### Manual bootstrap (auto-setup failed)

Set `REQ` to `<repo>/python/requirements-mcp.txt` or bundled copy. With mirrors:

```sh
export UV_INDEX_URL=https://pypi.tuna.tsinghua.edu.cn/simple   # if mainland
uv venv "$APP_DATA/python/.venv"
uv pip install -r "$REQ" --python "$APP_DATA/python/.venv/bin/python"
```

Windows: same with `$env:UV_INDEX_URL` and `Scripts\python.exe`.

### Verify core

```sh
uv --version
# managed venv:
python -c "import mcp, pandas; print('ok')"
```

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

## Layer 3 — Bioinformatics: pixi

**pixi** manages isolated per-project environments (conda + pip) — use for scanpy/single-cell, variant calling stacks, etc. The wisp **`python` tool** uses the core uv venv; run bioinfo code via **`shell`**: `pixi run python …` or `pixi run …` in the project directory.

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
| Corrupt core venv | Delete `python/.venv` under app data; restart (bootstrap recreates). |
| bear-* skill stops at CLI check | Install Node + `scimaster-cli` + `sci init`; do not fake citations. |

## Agent workflow

1. `use_skill` this file when Capabilities or bootstrap reports missing tools.
2. **Step 0a first** — detect mainland vs international; configure mirrors before any download.
3. Detect OS — PowerShell on Windows, `sh` elsewhere.
4. Install missing layers in order: **core (uv)** → **literature (Node+sci)** → **bioinfo (pixi)** as needed.
5. Verify each layer; tell user to **restart wisp-science** after PATH/config changes.
6. Finish with **attempt_completion**: region/mirror choice, what was installed, paths checked, Capabilities expectations.

## Not in scope

- Remote GPU / SSH / Modal → `compute-env-setup`
- Replacing pixi with conda/micromamba when pixi suffices locally
- SciMaster API billing / key provisioning beyond pointing to `sci init`
