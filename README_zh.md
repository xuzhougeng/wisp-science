# Wisp Science — 本地优先的 AI 科研工作台

[English](README.md) | [简体中文](README_zh.md)

<p>
<a href="https://github.com/xuzhougeng/wisp-science/releases"><img src="https://img.shields.io/github/downloads/xuzhougeng/wisp-science/total" alt="下载量"></a>
<a href="https://doi.org/10.5281/zenodo.21193742"><img src="https://zenodo.org/badge/1285857639.svg" alt="DOI"></a>
<a href="https://github.com/xuzhougeng/wisp-science/releases"><img src="https://img.shields.io/badge/Windows-supported-0078D4" alt="支持 Windows"></a>
<a href="https://github.com/xuzhougeng/wisp-science/releases"><img src="https://img.shields.io/badge/macOS-supported-000000" alt="支持 macOS"></a>
<a href="#构建与运行"><img src="https://img.shields.io/badge/Linux-source%20build-FCC624" alt="Linux 源码构建"></a>
<a href="https://github.com/xuzhougeng/wisp-science/blob/main/LICENSE"><img src="https://img.shields.io/github/license/xuzhougeng/wisp-science" alt="许可证"></a>
<br>
<a href="https://github.com/xuzhougeng/wisp-science/stargazers"><img src="https://img.shields.io/github/stars/xuzhougeng/wisp-science?style=social" alt="Stars"></a>
</p>

**Wisp Science** 是一个开源、本地优先的桌面 AI 科研助手和科学计算工作台。
它可连接兼容 OpenAI 或 Anthropic 的模型，在本地、SSH、WSL 和 GPU 计算环境
中运行持久化 Python 与 R，加载可复用的 Agent Skills（`SKILL.md`），并通过
内置 Model Context Protocol（MCP）服务访问约 80 个生物信息学与计算生物学
数据库。

Wisp Science 使用 Rust、Tauri v2 和 Leptos 构建，可作为跨平台桌面应用或
无界面 CLI 运行。

> **我们的宣言：** Wisp Science 开源、无国界。我们希望打造一个任何地方的
> 任何人都能使用、研究、改进和分享的科学工作台。

> 当前状态：MVP 垂直切片。Agent 循环、流式模型提供商、工具、Python/R
> REPL、SQLite 存储、MCP 客户端和 Leptos UI 均可构建并运行。尚未完成的
> 内容见[路线图](#路线图mvp-之后)。

## 目录结构

```text
wisp-science/
├─ crates/
│  ├─ wisp-llm/     Provider trait + OpenAI-compatible + Anthropic + SSE + RoutedProvider
│  ├─ wisp-core/    ContextManager（三层压缩）、SystemPrompt、agent_loop、memory
│  ├─ wisp-tools/   read/write/edit/search/grep/shell/attempt_completion + Windows 安全机制
│  ├─ wisp-store/   sqlx SQLite（projects/frames/messages/artifacts/settings）+ OS keyring
│  ├─ wisp-skills/  SKILL.md 发现 + use_skill 工具（内置目录位于 skills/）
│  ├─ wisp-runtime/ 项目级 Python/R 运行时管理器 + REPL 工具
│  ├─ wisp-mcp/     stdio JSON-RPC MCP 客户端 + McpTool 适配器（内置 bio-tools）
│  ├─ wisp-acp/     外部编码智能体的 ACP v1 stdio 客户端
│  ├─ wisp-sync/    加密快照协议 + 可自行托管的中继服务
│  └─ wisp-cli/     `wisp-science` 无界面可执行程序
├─ src-tauri/       Tauri v2 桌面壳（命令 + Agent 事件流）
├─ ui/              Leptos CSR 前端（由 Trunk 构建，在 WebView2 中加载）
├─ python/          kernel_worker.py + 模拟 MCP 服务
├─ r/               可选的系统 R kernel worker（需要 jsonlite）
├─ skills/          内置 SKILL.md 目录（29 个科研工作流）
├─ mcp-servers/     内置 MCP 服务（bio-tools：约 80 个数据库客户端）
└─ seed/            内置演示会话（CRISPR / 酶 / 极端微生物 / 免疫治疗）
```

## 前置要求

- **Rust**（stable，1.88+）及 `wasm32-unknown-unknown`：
  `rustup target add wasm32-unknown-unknown`
- **uv**（Python 环境管理器）：<https://docs.astral.sh/uv/>
- 可选：PATH 中存在 **R** 的 `Rscript`，并安装 `jsonlite` 包，以使用持久化
  `r` 工具。Wisp 不会自动安装 R 包。
- **Trunk**（WASM 前端打包器）：`cargo install --locked trunk`
- **Tauri CLI v2**：`cargo install tauri-cli --version "^2"`
- **WebView2 Runtime**（仅 Windows）：Windows 10/11 通常已安装 Evergreen
  Runtime，安装程序会在缺失时获取它。较旧或损坏的 Runtime 仍可能阻止主窗口
  显示；遇到问题请参考下方的 [Windows 安装与启动故障排除](#windows-安装与启动故障排除)。
- **Xcode Command Line Tools**（仅 macOS）：`xcode-select --install`。macOS
  使用系统 WebKit，无需额外运行时。

## 构建与运行

### 无界面 CLI

```powershell
$env:WISP_API_KEY = "<your provider key>"
$env:WISP_PROVIDER = "openai"           # openai、openai_responses 或 anthropic
$env:WISP_MODEL     = "deepseek-v4-pro" # openai_responses: gpt-5.5；anthropic: claude-sonnet-5
cargo run -p wisp-cli
```

CLI 会自动加载内置的 `skills/` 目录，并接入内置 Python 和可选的系统 R REPL。
Python 首次运行时会在 `.wisp/python/.venv` 中创建 uv 虚拟环境；R 使用 PATH
中的 `Rscript`，并要求该 R 环境已安装 `jsonlite`。在桌面应用中，可以通过
Contexts 面板或 Agent 的 `set_runtime_interpreter` 工具，按执行上下文保存
Python 与 R 解释器路径。因此 `local`、WSL 和每台 SSH 服务器都可使用不同
环境，而无需依赖宿主环境变量。必要时该工具会重启当前项目对应的 REPL，
从而在不重启 Wisp 的情况下恢复失败的运行时；重启会清空该 REPL 的内存状态。
输入框底部的计算主机按钮会先打开固定的主机列表，其中 `Local` 位于已配置的
SSH 主机之前；只有选择某一台主机后，才会在右上角显示该上下文独立的环境信息
卡，包括探测摘要、Runtime/Run 数量及详情、探测和终端快捷操作。

**设置 → 凭据**除内置服务外，也支持添加自定义凭据。每条自定义凭据将服务名称
映射到技能或客户端实际读取的环境变量（例如 `METASO_API_KEY`）。名称和变量名
保存在本地设置中，秘密值只保存在系统钥匙串，并注入新启动的本地 Python 与
内置 MCP 进程；自定义凭据不会复制到 SSH/WSL 主机。

### 桌面应用

```powershell
cargo tauri dev      # 热更新：Trunk 提供 UI，Tauri 打开 WebView2
cargo tauri build    # 在 target/release/bundle 下生成 MSI/NSIS 安装程序
```

#### Windows 安装与启动故障排除

- 如果未签名的 MSI/NSIS 安装包被 Microsoft Defender SmartScreen 拦截，请确认
  安装包来自本仓库的 [GitHub Releases](https://github.com/xuzhougeng/wisp-science/releases)，
  然后选择 **更多信息 → 仍要运行**。
- 如果安装完成后主窗口一闪而过、不可见或只剩系统托盘图标，请先从托盘菜单
  **Quit** 彻底退出 Wisp Science，再从 Microsoft 官方
  [WebView2 下载页面](https://developer.microsoft.com/microsoft-edge/webview2/#download-section)
  下载与系统架构匹配的最新 **Evergreen Standalone Installer**，并以管理员身份运行，
  以更新或修复 Microsoft Edge WebView2 Runtime。这里要求使用受支持的最新
  Evergreen Runtime，不表示 Wisp Science 依赖某个固定的大版本。
- WebView2 安装完成后重新打开 Wisp Science；如果窗口仍未恢复，请重启 Windows
  后再试。问题仍存在时，请在 issue 中附上 `winver` 结果、WebView2 Runtime
  版本、安装包文件名和复现步骤；不要上传 API Key、Token、密码或私钥。

桌面开发固定使用 `1421` 端口，UI 测试使用 `1422`。对应的 Trunk 输出分别
隔离在 `ui/dist-dev` 与 `ui/dist-test`，发布构建继续使用 `ui/dist`，避免正在
运行的开发或测试服务器与 `cargo tauri build` 并发复制优化后的 WASM 文件。

当图片或 PDF 内容超出可见区域时，预览支持缩放和拖拽平移，包括 100% 缩放状态。

在 macOS 上使用相同命令（`cargo tauri build` 会在
`target/release/bundle` 下生成 `.app` 和 `.dmg`）。
`src-tauri/tauri.macos.conf.json` 会由 Tauri 自动合并，以跨平台的
`trunk build` 替代 PowerShell `beforeBuildCommand`。构建 Apple Silicon 与
Intel 通用二进制：

```bash
rustup target add x86_64-apple-darwin
cargo tauri build --target universal-apple-darwin
```

`.app`/`.dmg` 未签名，首次启动时需要右键选择“打开”，或在“系统设置 →
隐私与安全性”中允许运行。

桌面应用把 API 密钥存入操作系统密钥环，并把模型配置保存在
`.wisp/wisp.sqlite`（Settings → Models）。配置可指向远程 API 提供商，字段
说明见[模型配置](docs/model-configuration.md)。每轮模型/工具循环上限可在
**设置 → 常规 → 每轮最大 Agent 迭代次数**中调整（默认 100；0 表示不限制）。对话也会持久化到该 SQLite
数据库：每轮消息都会追加到当前会话 frame，重启后可恢复完整历史。无界面
CLI 继续使用 `.wisp/session.json`，便于迁移。

项目可在 Windows 与 macOS 之间迁移。在 Projects 页面使用项目卡片上的下载
操作导出版本化 ZIP，再在另一台电脑上选择 **Import project**。导入器会要求
选择父目录并创建新的项目目录，不会复用 Windows 盘符。详情及限制见
[项目迁移](docs/project-transfer.md)。

项目还支持设备间的显式同步。可在 **Settings → General** 中配置自托管中继，
或配置由百度网盘/坚果云桌面客户端管理的文件夹，然后在项目卡片上点击
**Sync now**。同步不会在后台运行，并且当任务、审批、审查或运行处于活动状态
时会拒绝启动。项目内容在到达任一后端之前均会加密；工作区文件按内容增量
上传。设置、设备码、冲突、路径行为、部署和限制见
[手动项目同步](docs/project-sync.md)或
[中文同步指南](docs/project-sync.zh-CN.md)。

### 本地 ACP Agents

Wisp 可以启动任何已安装、通过 stdio 使用稳定版 ACP v1 的本地 Agent。
这与 **Settings → Models** 中的 HTTP API 模型配置相互独立。

快速开始：

1. 安装 ACP 适配器，例如 Codex：
   `npm install -g @agentclientprotocol/codex-acp`
2. 打开 **Settings → Models → ACP Agents**，或在聊天模型选择器中点击
   **Add ACP Agent**。不要把 ACP 启动命令填入 HTTP 的 “Add model” 表单。
3. 设置 **Label**、**Command**（`codex-acp`、`npx` 或 `npx.cmd`）及
   **Arguments**（每行一个；使用 `npx` 时依次填写 `-y` 和
   `@agentclientprotocol/codex-acp`）。
4. 依次执行 **Save Agent** → **Test Connection**，如有提示则完成认证。
5. 选择该 Agent 后发送消息。如果当前会话已有消息，Wisp 会自动新建空会话，
   因为 ACP 无法重新绑定现有的对话历史。首条消息发出后，所选 Agent 会锁定。

不要在此处直接使用 `codex` 或 `claude`，它们并不提供 ACP。请使用
[`codex-acp`](https://github.com/agentclientprotocol/codex-acp) 或
[`claude-agent-acp`](https://github.com/agentclientprotocol/claude-agent-acp)
等适配器。

完整设置步骤、Claude 示例、Windows 注意事项和故障排除见
[docs/acp-agents.md](docs/acp-agents.md)。

### 编辑器引用与搜索

在桌面对话中输入 `@` 可附加已保存的产物、上传的文件、执行环境或语言运行时，
输入 `#` 可附加已保存的会话（包括其他项目的会话），输入 `/` 可让下一轮使用
已启用的 skill。附件会显示为可移除的显式标签；跨项目产物保留原始本地路径，
不会被自动复制。ACP Agent 同样支持这些引用：所选 skill 与会话上下文作为 ACP
文本块发送，产物则作为文件链接发送。

你上传的文件本身也是产物，因此一直都能通过 `@` 引用；它们带有 **Upload**
标记，以便与 agent 产出的文件区分。

图片预览提供区域框选工具。拖出区域后，Wisp 会持续高亮所选范围，并让你选择
“添加到对话”（留在预览）或“添加到对话并返回对话”；确认前不会把截图直接
附加到编辑器。

`@` 也可以指向算力。引用执行环境（`@CPU1`）会把本轮工作指向那台服务器，并
自动为该会话开启它，无需先到算力菜单里手动打开——本地算力始终可用，不需要
开关。引用运行时（`@runtime_R`）会指向该环境上常驻的 Python 或 R 会话，它在
多次调用之间保留变量，因此 agent 会直接查看当前状态，而不是重跑之前的工作。
只有配置或探测到相应解释器的环境才会出现运行时条目。引用运行时不会启动它。

在 Windows/Linux 上按 Ctrl+K，在 macOS 上按 Cmd+K，可以搜索项目、产物、
会话和常用命令。按 Enter 打开所选结果；按 Shift+Enter 将产物或会话附加到
编辑器。

编辑器默认按 Enter 发送、按 Shift+Enter 换行。你也可以在
**设置 → 常规 → 发送与换行快捷键**中改为 Windows/Linux 按 Ctrl+Enter、
macOS 按 Cmd+Enter 发送，此时按 Enter 直接换行。

在 macOS、Windows 和 Linux 的侧边栏中，已保存的会话和会话文件夹均提供
可见的操作按钮。可以重命名或删除文件夹，也可以重命名、整理、复制、移动、
导出或删除会话。远程文件行也提供可见的下载操作，同时仍可使用右键菜单。
打开会话时先加载最新 20 个完整用户轮次；顶部可继续加载更早消息。聊天区同时
最多挂载 40 个完整用户轮次，可通过“显示更早/较新的消息”在已加载历史间移动，
避免超长会话无限扩张 DOM。
跨项目转移只会复制已保存的对话文本；项目文件与运行仍留在源项目中，关联的
产物记录不会转移，底层工作区文件也不会被删除。

在 macOS 上，原生应用菜单包含全局桌面命令，包括项目导航、新会话、编辑
快捷键和 **Check for Updates…**。针对具体会话和文件夹的操作仍位于对应行。
Settings 页面以及 Windows 的窗口内 Help 菜单同样提供更新检查。结果会在
应用内对话框中显示，包括当前是否已是最新版，以及 GitHub Releases 上是否
存在新版本。

## 配置

以下配置均为可选，项目提供了合理的默认值。

| 变量 | 用途 |
|---|---|
| `WISP_API_KEY` | 模型提供商 API 密钥（CLI）；桌面端改用密钥环 |
| `WISP_PROVIDER` | CLI API 提供商：`openai`（默认）、`openai_responses` 或 `anthropic` |
| `WISP_API_URL` | API 根地址；默认使用 DeepSeek / OpenAI / Anthropic |
| `WISP_MODEL` | 模型名称 |
| `WISP_MAX_CONTEXT` | 上下文预算（默认 1,000,000） |
| `WISP_MAX_ITER` | 每轮 Agent 最大迭代次数（默认 100；0 表示不限制） |
| `WISP_SKILLS_PATH` | 额外的 SKILL.md 目录，以 `;` 或 `:` 分隔 |
| `WISP_KERNEL_WORKER` | 覆盖内置 `kernel_worker.py` 路径 |
| `WISP_MCP_COMMAND` | 启动任意 stdio MCP 服务（完整命令行） |
| `WISP_MCP_PKG` | 启动内置 bio-tools 服务，例如 `mcp_pubmed` |

### 内置 bio-tools MCP

`WISP_MCP_PKG=mcp_pubmed` 会在 uv 虚拟环境中启动
`mcp-servers/bio-tools/run_server.py mcp_pubmed`。需要先在该环境中安装服务
依赖：

```powershell
uv pip install mcp requests
# 以及该服务导入的专用依赖，例如 httpx、xmltodict 等
```

之后 Agent 即可直接调用该服务的工具，例如 PubMed 搜索。

### Notion MCP

Notion 使用与其他托管 MCP 服务相同的远程 URL 流程。进入
**设置 → 连接 → 添加连接**，选择**远程 URL**，填写
`https://mcp.notion.com/mcp`，将**认证方式**设为 **OAuth**，再点击
**测试**或**保存**。两项操作都会在浏览器中打开 Notion 授权页；测试会验证连接
并获取工具列表，但不会保存连接；保存只会在授权成功后写入连接。启动应用时不会
自动创建或认证 Notion 连接。

OAuth 访问令牌和刷新令牌保存在系统密钥环中，不会写入项目数据库。删除已保存的
连接时也会清除对应凭据。连接详情会显示服务 URL、启用状态和 OAuth 认证方式。

该连接会用于之后新建的 Agent 会话。Agent 可访问的内容由 Notion 工作区权限决定，
请在批准写入操作前仔细确认。

### 内置演示

`seed/` 提供四个预先录制的示例会话：CRISPR 筛选、酶工程、极端微生物和
免疫治疗。在桌面应用中，**Open demo** 会列出这些示例，并以只读的 User +
Assistant 对话形式打开。打开时会把内置的 `assets_*.tar.gz` 解压到工作区，
因此右侧面板可以正确预览图像和数据文件。

## 测试

- **Rust 单元测试**：`cargo test --workspace`，覆盖 `wisp-store` SQLite
  往返读写、seed 演示加载器等。
- **MCP 客户端冒烟测试**：`cargo run -p wisp-mcp --example smoke`，通过
  `uv` 启动内置模拟 MCP 服务，并完成 `tools/list` 与 `tools/call` 往返调用。
- **UI E2E（Playwright + Tauri mock）**：`ui-tests/` 在无头浏览器中运行
  Leptos UI，并使用模拟的 `window.__TAURI__`，因此不需要 Rust 后端或 API
  密钥：

  ```powershell
  cd ui-tests
  npm install
  npx playwright install chromium      # 仅首次需要下载浏览器
  npx playwright test                  # 启动 UI 并运行完整模拟桌面流程测试
  ```

  模拟实现位于 `tests/mock-tauri.ts`，它会使用固定数据替代 `invoke`/`listen`，
  并模拟流式 Assistant 回复。因此测试能够覆盖真实的 Leptos 渲染与事件处理，
  同时不访问网络。

## 架构

- **Agent 循环**（`wisp-core::agent`）：读取 → 思考 → 工具调用 → 验证；token
  会流式发送到 `Output`。调用 `attempt_completion` 或模型不再返回工具调用时
  停止。
- **上下文压缩**（`wisp-core::context`）：每次模型调用前，当上下文达到预算的
  80% 时触发三层处理——微压缩过大的工具输出、丢弃较旧轮次，最后才使用
  LLM 生成完整摘要。
- **模型提供商**（`wisp-llm`）：一个 trait、两种 wire format（OpenAI
  `/chat/completions` 与 Anthropic `/v1/messages`），均支持 SSE 流式输出。
  `RoutedProvider` 根据最后一条用户消息选择 low/medium/high 层级。
- **工具**（`wisp-tools`）：文件系统与 shell 工具，提供 Windows 感知的危险
  命令门控，并使用 `dunce` 规范化路径，将沙箱限制在项目目录内。
- **Python/R REPL**（`wisp-runtime`）：每个项目、执行上下文和语言各有一个由
  manager 管理的进程，可跨 cell 和会话保持命名空间。local、WSL 和 SSH 上下文
  使用同一个版本化协议。R 是可选功能，使用现有 `Rscript` 和 `jsonlite`。
  Contexts 面板可探测解释器能力，并显示运行时状态、内存、最后活动时间、具有
  破坏性的 Stop/Restart 控件，以及按需只读展示的内存对象名、类型、形状/大小
  和有限元数据。
- **MCP**（`wisp-mcp`）：最小化的 newline-JSON-RPC 客户端，可启动任意 stdio
  MCP 服务，并把每个远程工具作为一等 Agent 工具公开。

## 致谢

- **Claude Science (Operon)** 用于产品对比与兼容性研究。
- Agent 核心基于
  [`w4n9H/mangopi-cli`](https://github.com/w4n9H/mangopi-cli)（Apache-2.0）。
- `skills/` 与 `mcp-servers/bio-tools/` 来自上游 `wisp-science` 资源包
  （Apache-2.0）。
- `skills/bear-*` 来自
  [bear-research-skills](https://github.com/fei0810/bear-research-skills)
  （CC BY-NC-SA 4.0）；在线检索需要 `scimaster-cli`。
- `kernels/kernel_worker.py` 协议改编自上游 operon kernel worker；为支持
  Windows，移除了仅适用于 POSIX 的 `resource`、`/proc` 和 `SIGINT` 机制。

许可证见 `LICENSE`（Apache-2.0）。上游声明保留在各自目录中。

## 引用

如果你在研究中使用 wisp-science，请引用：

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

## 路线图（MVP 之后）

- `FlashThinking`：按阶段注入结构化思考框架。
- `loop_engine`：在当前有界自动 Reviewer 流程之外，提供更深入的
  Implementer / Verifier / Updater 工作流。
- 产物管理，以及 UI 中的内嵌 Mol* 三维结构查看器。
- `RoutedProvider` 基于 LLM 评分选择层级（基于关键词的选择已接入）。
- 将 `skills/` 和 `mcp-servers/` 打包到 Tauri 安装程序，使发布包无需源码树
  即可完整运行。

## Star History

<a href="https://star-history.com/#xuzhougeng/wisp-science&Date">
  <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=xuzhougeng/wisp-science&type=Date" />
</a>
